//! Parallel encode scaling for buffers around and above the last-level cache.
//!
//! Run with `taskset -c 4-7` so two and four workers can use distinct physical
//! cores. The single-thread row is the same process and the same buffers, so
//! the comparison is not affected by a different binary layout.
//!
//! ```sh
//! cargo build --release --manifest-path benchmarks/compare/Cargo.toml \
//!   --bin parallel_encode
//! taskset -c 4-7 benchmarks/compare/target/release/parallel_encode
//! ```

use std::hint::black_box;
use std::time::Instant;

const SIZES: &[usize] = &[
    2 * 1024 * 1024,
    3 * 1024 * 1024,
    4 * 1024 * 1024,
    5 * 1024 * 1024,
    6 * 1024 * 1024,
    6 * 1024 * 1024 + 512 * 1024,
    7 * 1024 * 1024,
    8 * 1024 * 1024,
    12 * 1024 * 1024,
    16 * 1024 * 1024,
    24 * 1024 * 1024,
    32 * 1024 * 1024,
];
const THREADS: &[usize] = &[1, 2, 4, 0];
const ROUNDS: usize = 5;

struct Aligned {
    mem: Vec<u8>,
    start: usize,
    len: usize,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let mem = vec![0u8; len + 64];
        let start = (64 - mem.as_ptr() as usize % 64) % 64;
        Self { mem, start, len }
    }

    fn slice(&self) -> &[u8] {
        &self.mem[self.start..self.start + self.len]
    }

    fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.mem[self.start..self.start + self.len]
    }
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

/// Sparse fingerprint: 256 bytes spread across the buffer. It is cheap enough
/// not to disturb the cache and still catches gross output corruption.
fn fingerprint(bytes: &[u8]) -> [u8; 256] {
    let mut out = [0u8; 256];
    for (i, slot) in out.iter_mut().enumerate() {
        *slot = bytes[i * bytes.len() / 256];
    }
    out
}

fn main() {
    let mut thread_filter: Option<usize> = None;
    let mut size_filter: Option<usize> = None;
    let mut out_prefix = String::from("parallel_encode");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--threads" => thread_filter = args.next().and_then(|v| v.parse().ok()),
            "--size" => size_filter = args.next().and_then(|v| v.parse().ok()),
            "--out" => out_prefix = args.next().expect("missing --out value"),
            other => panic!("unknown argument: {other}"),
        }
    }
    let threads: Vec<usize> = THREADS
        .iter()
        .copied()
        .filter(|&t| thread_filter.is_none_or(|f| f == t))
        .collect();
    if threads.is_empty() {
        panic!("no matching thread count");
    }
    let sizes: Vec<usize> = SIZES
        .iter()
        .copied()
        .filter(|&s| size_filter.is_none_or(|f| f == s))
        .collect();
    if sizes.is_empty() {
        panic!("no matching size");
    }

    let mut raw = String::from("size,threads,round,ns,gbps,tsc_cycles_per_byte\n");
    let mut summary = String::from("size,threads,median_ns,gbps,tsc_cycles_per_byte,speedup\n");

    for &size in &sizes {
        let mut input = Aligned::new(size);
        for (i, byte) in input.slice_mut().iter_mut().enumerate() {
            *byte = (i * 73 + 19) as u8;
        }
        let cap = yolow64::STANDARD.encoded_len(size).unwrap();
        let mut output = Aligned::new(cap);
        let mut reference = Aligned::new(cap);

        let reference_len = yolow64::STANDARD
            .encode(input.slice(), reference.slice_mut())
            .unwrap();
        assert_eq!(reference_len, cap);
        let reference_fingerprint = fingerprint(&reference.slice()[..cap]);
        // Keep the reference out of the timed working set: it is only needed
        // to establish the expected fingerprint once.
        drop(reference);

        let iters = (256 * 1024 * 1024 / size).clamp(2, 64);
        let mut series: Vec<Vec<f64>> =
            threads.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();
        let mut cycles: Vec<Vec<f64>> =
            threads.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();

        for round in 0..ROUNDS {
            for step in 0..threads.len() {
                let idx = (step + round) % threads.len();
                let threads = threads[idx];
                let start_t = Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    let written = if threads == 0 {
                        yolow64::STANDARD
                            .encode_parallel(
                                black_box(input.slice()),
                                black_box(output.slice_mut()),
                            )
                            .unwrap()
                    } else if threads == 1 {
                        yolow64::STANDARD
                            .encode(black_box(input.slice()), black_box(output.slice_mut()))
                            .unwrap()
                    } else {
                        yolow64::STANDARD
                            .encode_parallel_with_threads(
                                black_box(input.slice()),
                                black_box(output.slice_mut()),
                                threads,
                            )
                            .unwrap()
                    };
                    black_box(written);
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                // Stop the wall clock before the output check so the check is
                // not part of the measured encode time.
                let elapsed = start_t.elapsed();
                assert_eq!(fingerprint(&output.slice()[..cap]), reference_fingerprint);
                let ns = elapsed.as_secs_f64() * 1e9 / iters as f64;
                let tsc = (end_c.wrapping_sub(start_c)) as f64 / iters as f64 / size as f64;
                series[idx].push(ns);
                cycles[idx].push(tsc);
                raw.push_str(&format!(
                    "{size},{threads},{round},{ns:.4},{:.4},{tsc:.6}\n",
                    size as f64 / ns
                ));
            }
        }

        let base = median(&mut series[0].clone());
        for (idx, &thread_count) in threads.iter().enumerate() {
            let ns = median(&mut series[idx]);
            let tsc = median(&mut cycles[idx]);
            summary.push_str(&format!(
                "{size},{thread_count},{ns:.4},{:.4},{tsc:.6},{:.4}\n",
                size as f64 / ns,
                base / ns
            ));
        }
    }

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{out_prefix}_raw.csv")), raw).unwrap();
    std::fs::write(dir.join(format!("{out_prefix}_summary.csv")), summary).unwrap();
    eprintln!("wrote {}", dir.display());
}
