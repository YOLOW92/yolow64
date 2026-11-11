//! Adaptive multi-threaded encode/decode.
//!
//! Small inputs stay on the single-threaded path. Large inputs are split at
//! Base64 group boundaries and processed with `std::thread::scope`; the number
//! of workers is chosen from the input size and available parallelism.

use crate::{Codec, Error};

/// Inputs below this size are cheaper to process on one thread. The 4 MiB
/// encode crossover is measured: 2 MiB is faster single-threaded, while 4 and
/// 6 MiB gain from four cache-resident workers.
const MIN_PARALLEL_ENCODE: usize = 4 * 1024 * 1024;
const MIN_PARALLEL_DECODE: usize = 2 * 1024 * 1024;

/// Aim for at least this much data per worker.
const TARGET_CHUNK: usize = 1024 * 1024;

/// Encode gains from up to four workers while the working set is cache
/// resident. Above the cache limit the single-thread streaming-store path wins,
/// so the policy falls back to one worker.
const MAX_ENCODE_WORKERS: usize = 4;

/// Decode is measured separately and still uses the conservative two-worker
/// policy until its own crossover round.
const MAX_DECODE_WORKERS: usize = 2;

/// Working-set limit for the auto policy. Parallel workers help while the
/// input and output stay cache-resident; beyond that the codec is memory-bound
/// and extra threads mostly add contention. On unknown systems use a
/// conservative value.
const DEFAULT_CACHE_LIMIT: usize = 8 * 1024 * 1024;

/// Last-level cache size from CPUID on x86_64, or sysfs on other Linux
/// architectures. `None` means scheduling uses the conservative fallback.
fn detected_llc() -> Option<usize> {
    static LLC: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    *LLC.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        {
            crate::llc_bytes()
        }
        #[cfg(all(not(target_arch = "x86_64"), target_os = "linux"))]
        {
            linux_llc_bytes()
        }
        #[cfg(all(not(target_arch = "x86_64"), not(target_os = "linux")))]
        {
            None
        }
    })
}

#[cfg(all(not(target_arch = "x86_64"), target_os = "linux"))]
fn linux_llc_bytes() -> Option<usize> {
    for cpu in 0..16 {
        let path = format!("/sys/devices/system/cpu/cpu{cpu}/cache/index3/size");
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let text = text.trim();
        let (number, scale) = if let Some(n) = text.strip_suffix('K') {
            (n, 1024usize)
        } else if let Some(n) = text.strip_suffix('M') {
            (n, 1024 * 1024)
        } else if let Some(n) = text.strip_suffix('G') {
            (n, 1024 * 1024 * 1024)
        } else {
            (text, 1usize)
        };
        if let Ok(n) = number.parse::<usize>() {
            let total = n.saturating_mul(scale);
            if total >= 1024 * 1024 {
                return Some(total);
            }
        }
    }
    None
}

/// Decode keeps a safety margin: its working set estimate is less exact
/// because padding and whitespace can change the chunking.
fn cache_limit() -> usize {
    detected_llc()
        .map(|bytes| bytes.saturating_mul(7) / 8)
        .unwrap_or(DEFAULT_CACHE_LIMIT)
}

/// Encode can use the full LLC: the parallel chunks are split at group
/// boundaries and the last chunk owns the padding, so the working set is
/// exactly `input + 4/3 input`. Above this limit the single-thread
/// streaming-store path is faster than cache-thrashing workers.
fn encode_cache_limit() -> usize {
    detected_llc().unwrap_or(DEFAULT_CACHE_LIMIT)
}

/// `available_parallelism` can issue a syscall; the process's CPU count does
/// not change, so cache it once. Without this, the wrapper cost is visible on
/// 2–3 MiB calls even when the policy returns one worker.
fn available_parallelism() -> usize {
    static AVAILABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    })
}

impl Codec {
    fn parallel_workers(
        self,
        len: usize,
        output_len: usize,
        min_len: usize,
        max_workers: usize,
        cache_limit: usize,
    ) -> usize {
        self.parallel_workers_for(
            len,
            output_len,
            min_len,
            max_workers,
            cache_limit,
            available_parallelism(),
        )
    }

    /// Pure worker-count policy, separated from `available_parallelism` so the
    /// cache and size crossovers can be unit-tested on any machine.
    fn parallel_workers_for(
        self,
        len: usize,
        output_len: usize,
        min_len: usize,
        max_workers: usize,
        cache_limit: usize,
        available: usize,
    ) -> usize {
        let max_workers = max_workers.min(available);
        if max_workers < 2 || len < min_len || len.saturating_add(output_len) > cache_limit {
            return 1;
        }
        // At least two chunks once the threshold is crossed, otherwise one
        // chunk per target size.
        max_workers.min(len.div_ceil(TARGET_CHUNK).max(2))
    }

    /// Encode using multiple threads when the input is large enough.
    ///
    /// The output bytes are identical to [`Codec::encode`]. Small inputs use
    /// the single-threaded implementation directly.
    pub fn encode_parallel(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        // Below the measured crossover the wrapper would only add work; call
        // the single-threaded encoder directly.
        if input.len() < MIN_PARALLEL_ENCODE {
            return self.encode(input, output);
        }
        self.encode_parallel_impl(
            input,
            output,
            MAX_ENCODE_WORKERS,
            MIN_PARALLEL_ENCODE,
            encode_cache_limit(),
        )
    }

    fn encode_parallel_impl(
        self,
        input: &[u8],
        output: &mut [u8],
        max_workers: usize,
        min_len: usize,
        cache_limit: usize,
    ) -> Result<usize, Error> {
        let len = self.encoded_len(input.len()).ok_or(Error::InvalidLength)?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        let workers = self.parallel_workers(input.len(), len, min_len, max_workers, cache_limit);
        if workers <= 1 {
            return self.encode(input, output);
        }

        let groups = input.len() / 3;
        let workers = workers.min(groups.max(1));
        if workers <= 1 {
            return self.encode(input, output);
        }

        // Every chunk except the last contains whole 3-byte groups, so its
        // output has no padding. The last chunk also receives the remainder.
        // Round the common chunk down to a multiple of four groups: four
        // groups produce 16 output bytes, so every worker after the first
        // starts on a 16-byte boundary and the streaming-store path stays
        // eligible even when the input itself is not 16-byte aligned.
        let base = (groups / workers) & !3;
        if base == 0 {
            return self.encode(input, output);
        }
        let mut ranges = Vec::with_capacity(workers);
        let mut in_at = 0usize;
        let mut out_at = 0usize;
        for i in 0..workers {
            let chunk_groups = if i + 1 == workers {
                groups - base * (workers - 1)
            } else {
                base
            };
            let in_len = chunk_groups * 3;
            let out_len = chunk_groups * 4;
            if i + 1 == workers {
                ranges.push((in_at, input.len(), out_at, len));
            } else {
                ranges.push((in_at, in_at + in_len, out_at, out_at + out_len));
            }
            in_at += in_len;
            out_at += out_len;
        }

        let mut out_chunks = Vec::with_capacity(workers);
        let mut rest = &mut output[..len];
        for &(_, _, out_start, out_end) in &ranges[..ranges.len() - 1] {
            let (chunk, tail) = rest.split_at_mut(out_end - out_start);
            out_chunks.push(chunk);
            rest = tail;
        }
        out_chunks.push(rest);

        let mut results = Vec::with_capacity(workers);
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            for (&(in_start, in_end, _, _), out_chunk) in ranges.iter().zip(out_chunks) {
                let input = &input[in_start..in_end];
                handles.push(scope.spawn(move || self.encode(input, out_chunk)));
            }
            for handle in handles {
                results.push(handle.join().unwrap_or(Err(Error::InvalidLength)));
            }
        });

        let mut written = 0usize;
        for result in results {
            written += result?;
        }
        Ok(written)
    }

    /// Like [`Codec::encode_parallel`], but caps the worker count.
    ///
    /// `threads` is clamped to the number of available CPUs. A value of one
    /// selects the single-threaded encoder.
    pub fn encode_parallel_with_threads(
        self,
        input: &[u8],
        output: &mut [u8],
        threads: usize,
    ) -> Result<usize, Error> {
        self.encode_parallel_impl(input, output, threads.max(1), 1, usize::MAX)
    }

    /// Decode using multiple threads when the input is large enough.
    ///
    /// Whitespace-tolerant decoding currently uses the single-threaded path,
    /// because whitespace changes the character grid before chunks can be
    /// split at quartet boundaries.
    pub fn decode_parallel(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        if input.len() < MIN_PARALLEL_DECODE {
            return self.decode(input, output);
        }
        self.decode_parallel_impl(
            input,
            output,
            MAX_DECODE_WORKERS,
            MIN_PARALLEL_DECODE,
            cache_limit() / 2,
        )
    }

    fn decode_parallel_impl(
        self,
        input: &[u8],
        output: &mut [u8],
        max_workers: usize,
        min_len: usize,
        cache_limit: usize,
    ) -> Result<usize, Error> {
        if self.ignore_whitespace {
            return self.decode(input, output);
        }
        let len = self.allocation_len(input)?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }

        // Keep the final partial/padded quartet out of the parallel region so
        // every worker gets whole quartets with no padding.
        let (bulk_len, tail_len) = if self.padding {
            if input.len() >= 2 && input[input.len() - 2..].contains(&b'=') {
                (input.len().saturating_sub(4), 4)
            } else {
                (input.len(), 0)
            }
        } else {
            let rem = input.len() % 4;
            if rem == 1 {
                return Err(Error::InvalidLength);
            }
            (input.len() - rem, rem)
        };

        let workers = self.parallel_workers(
            bulk_len,
            bulk_len / 4 * 3,
            min_len,
            max_workers,
            cache_limit,
        );
        let quads = bulk_len / 4;
        let workers = workers.min(quads.max(1));
        if workers <= 1 {
            return self.decode(input, output);
        }

        let base = quads / workers;
        let extra = quads % workers;
        let mut ranges = Vec::with_capacity(workers);
        let mut in_at = 0usize;
        let mut out_at = 0usize;
        for i in 0..workers {
            let chunk_quads = base + usize::from(i < extra);
            let in_len = chunk_quads * 4;
            let out_len = chunk_quads * 3;
            ranges.push((in_at, in_at + in_len, out_at, out_at + out_len));
            in_at += in_len;
            out_at += out_len;
        }

        let mut out_chunks = Vec::with_capacity(workers);
        let mut rest = &mut output[..out_at];
        for &(_, _, out_start, out_end) in &ranges[..ranges.len() - 1] {
            let (chunk, tail) = rest.split_at_mut(out_end - out_start);
            out_chunks.push(chunk);
            rest = tail;
        }
        out_chunks.push(rest);

        let mut results = Vec::with_capacity(workers);
        std::thread::scope(|scope| {
            let mut handles = Vec::with_capacity(workers);
            for (&(in_start, in_end, _, _), out_chunk) in ranges.iter().zip(out_chunks) {
                let input = &input[in_start..in_end];
                handles.push(
                    scope.spawn(move || {
                        self.decode(input, out_chunk).map_err(|err| (in_start, err))
                    }),
                );
            }
            for handle in handles {
                results.push(handle.join().unwrap_or(Ok(0)));
            }
        });

        for result in results {
            match result {
                Ok(_) => {}
                Err((base, Error::InvalidByte { byte, offset })) => {
                    return Err(Error::InvalidByte {
                        byte,
                        offset: base + offset,
                    });
                }
                Err((_, err)) => return Err(err),
            }
        }

        if tail_len != 0 {
            let prefix = bulk_len / 4 * 3;
            self.decode(&input[bulk_len..], &mut output[prefix..len])
                .map_err(|err| match err {
                    Error::InvalidByte { byte, offset } => Error::InvalidByte {
                        byte,
                        offset: bulk_len + offset,
                    },
                    other => other,
                })?;
        }
        Ok(len)
    }

    /// Like [`Codec::decode_parallel`], but caps the worker count.
    ///
    /// `threads` is clamped to the number of available CPUs. A value of one
    /// selects the single-threaded decoder.
    pub fn decode_parallel_with_threads(
        self,
        input: &[u8],
        output: &mut [u8],
        threads: usize,
    ) -> Result<usize, Error> {
        self.decode_parallel_impl(input, output, threads.max(1), 1, usize::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_worker_policy_follows_the_cache_crossover() {
        let codec = crate::STANDARD;
        let llc = 16 * 1024 * 1024usize;
        let workers = |len: usize, available: usize| {
            codec.parallel_workers_for(
                len,
                codec.encoded_len(len).unwrap(),
                MIN_PARALLEL_ENCODE,
                4,
                llc,
                available,
            )
        };

        // 3 MiB: below the measured 4 MiB crossover.
        assert_eq!(workers(3 << 20, 8), 1);
        // 4 MiB: cache-resident, four workers.
        assert_eq!(workers(4 << 20, 8), 4);
        // 6.5 MiB: working set is still within the 16 MiB LLC.
        assert_eq!(workers(6_815_744, 8), 4);
        // 7 MiB: working set exceeds the LLC; the NT path is faster alone.
        assert_eq!(workers(7 << 20, 8), 1);
        // 16 MiB: far above the LLC.
        assert_eq!(workers(16 << 20, 8), 1);
        // A two-core machine still gets two workers in the cache-resident band.
        assert_eq!(workers(4 << 20, 2), 2);
    }
}
