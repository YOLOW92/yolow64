//! Base64 encoding methods and data URL helper.

use std::mem::MaybeUninit;

#[cfg(target_arch = "x86_64")]
use crate::avx2;
#[cfg(target_arch = "x86_64")]
use crate::cpu::{avx2_available, stream_floor};
#[cfg(target_arch = "aarch64")]
use crate::cpu::neon_available;
#[cfg(target_arch = "aarch64")]
use crate::neon;
use crate::tables::{STANDARD_ALPHABET, STANDARD_PAIRS, URL_ALPHABET, URL_PAIRS};
use crate::{Alphabet, Codec, Error, STANDARD, maybe_uninit_slice};

impl Codec {
    /// Exact output length, or `None` if it cannot fit in a `usize`.
    #[inline]
    pub fn encoded_len(self, len: usize) -> Option<usize> {
        // 规律 2：3 个输入字节对应 4 个输出字符。
        let full = (len / 3).checked_mul(4)?;
        let tail = match (len % 3, self.padding) {
            (0, _) => 0,
            (_, true) => 4,
            (n, false) => n + 1,
        };
        full.checked_add(tail)
    }

    /// Encode into a caller-provided buffer.
    ///
    /// This is the default in-memory policy: the output stays cacheable below
    /// the last-level-cache crossover, which is best when the caller reuses or
    /// rereads the buffer.
    pub fn encode(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        #[cfg(target_arch = "x86_64")]
        if matches!(input.len(), 8 | 16 | 32) && self.padding && avx2_available() {
            #[cfg(test)]
            let short_variant =
                avx2::lab::ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
            #[cfg(not(test))]
            let short_variant = 0u8;
            if short_variant != 8 {
                let len = match input.len() {
                    8 => 12,
                    16 => 24,
                    _ => 44,
                };
                if output.len() < len {
                    return Err(Error::OutputTooSmall);
                }
                // SAFETY: AVX2 was detected above and padded 8/16-byte input
                // has exactly 12/24 output bytes.
                unsafe {
                    match input.len() {
                        8 => avx2::encode_8(input, maybe_uninit_slice(output), self.alphabet),
                        16 => avx2::encode_16(input, maybe_uninit_slice(output), self.alphabet),
                        _ => avx2::encode_32(input, maybe_uninit_slice(output), self.alphabet),
                    }
                };
                return Ok(len);
            }
        }
        self.encode_uninit(input, maybe_uninit_slice(output))
    }

    /// Encode for a one-shot consumer such as a socket, file or compressor.
    ///
    /// From 4 KiB upward this uses non-temporal stores when the output is
    /// 16-byte aligned, so the output is written once without a
    /// read-for-ownership. Use [`Codec::encode`] instead when the same output
    /// buffer is reused or immediately reread from cache.
    pub fn encode_streaming(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        self.encode_uninit_with(input, maybe_uninit_slice(output), true)
    }

    #[inline]
    pub(crate) fn encode_uninit(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        self.encode_uninit_with(input, output, false)
    }

    /// Encode to a writer using a fixed 64 KiB output buffer.
    ///
    /// The input is processed in 48 KiB multiples of three, so only the final
    /// call emits padding. Peak library memory is one 64 KiB buffer instead of
    /// the full encoded output. This is the memory-bounded API for files and
    /// sockets; wrap the writer in a `BufWriter` if it does many small writes.
    pub fn encode_to_writer<W: std::io::Write>(
        self,
        input: &[u8],
        writer: &mut W,
    ) -> std::io::Result<()> {
        const CHUNK: usize = 48 * 1024;
        let cap = self
            .encoded_len(CHUNK)
            .ok_or_else(|| std::io::Error::other("encoded length overflow"))?;
        let mut buffer = Vec::with_capacity(cap);
        let bulk = input.len() / 3 * 3;
        let mut at = 0usize;
        while bulk - at >= CHUNK {
            let spare = buffer.spare_capacity_mut();
            let written = self
                .encode_uninit(&input[at..at + CHUNK], spare)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
            // SAFETY: `encode_uninit` initialized `written` bytes of the spare
            // capacity and `written` is the exact encoded length.
            unsafe { buffer.set_len(written) };
            writer.write_all(&buffer)?;
            buffer.clear();
            at += CHUNK;
        }
        let remaining = bulk - at;
        if remaining != 0 {
            let spare = buffer.spare_capacity_mut();
            let written = self
                .encode_uninit(&input[at..at + remaining], spare)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
            // SAFETY: as above.
            unsafe { buffer.set_len(written) };
            writer.write_all(&buffer)?;
            buffer.clear();
            at += remaining;
        }
        if at < input.len() {
            let spare = buffer.spare_capacity_mut();
            let written = self
                .encode_uninit(&input[at..], spare)
                .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidInput, err))?;
            // SAFETY: as above.
            unsafe { buffer.set_len(written) };
            writer.write_all(&buffer)?;
        }
        Ok(())
    }

    pub(crate) fn encode_uninit_with(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
        streaming: bool,
    ) -> Result<usize, Error> {
        let len = self.encoded_len(input.len()).ok_or(Error::InvalidLength)?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        let mut out = 0;
        let mut start = 0;
        #[cfg(target_arch = "x86_64")]
        if (12..24).contains(&input.len()) && avx2_available() {
            #[cfg(test)]
            let short_variant =
                avx2::lab::ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
            #[cfg(not(test))]
            let short_variant = 0u8;
            if input.len() == 16 && self.padding && short_variant != 8 {
                // SAFETY: runtime detection guarantees AVX2; padded 16-byte
                // input has exactly 24 output bytes, checked above.
                unsafe { avx2::encode_16(input, output, self.alphabet) };
                start = 16;
                out = 24;
            } else {
                // SAFETY: runtime detection guarantees AVX2 and the exact
                // output length was checked above; 16 bytes are available for
                // the first 12 input bytes.
                unsafe { avx2::encode_12(input, output, self.alphabet) };
                start = 12;
                out = 16;
            }
        }
        #[cfg(target_arch = "x86_64")]
        if input.len() >= 24 && avx2_available() {
            // SAFETY: runtime detection guarantees AVX2 and `output` has the
            // exact required encoded length checked above. The two-load kernel
            // avoids cache-line-crossing 32-byte loads on short inputs; the
            // single-load kernel has fewer uops for larger inputs.
            #[cfg(test)]
            let variant = avx2::lab::ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
            #[cfg(not(test))]
            let variant = 0u8;
            start = match variant {
                1 => unsafe { avx2::encode(input, output, self.alphabet) },
                #[cfg(test)]
                2 => unsafe { avx2::lab::encode_unroll8_seq(input, output, self.alphabet) },
                #[cfg(test)]
                3 => unsafe { avx2::lab::encode_countloop(input, output, self.alphabet) },
                4 => unsafe { avx2::encode_load16(input, output, self.alphabet) },
                5 => unsafe { avx2::encode_unroll8_split::<0>(input, output, self.alphabet) },
                #[cfg(test)]
                6 => {
                    debug_assert_eq!(output.as_mut_ptr() as usize % 32, 0);
                    unsafe { avx2::encode_unroll8_split::<1>(input, output, self.alphabet) }
                }
                #[cfg(test)]
                7 => {
                    debug_assert_eq!(output.as_mut_ptr() as usize % 16, 0);
                    unsafe { avx2::encode_unroll8_split::<2>(input, output, self.alphabet) }
                }
                _ => {
                    if input.len() <= 96 {
                        unsafe { avx2::encode_load16(input, output, self.alphabet) }
                    } else if input.len() >= 4096 {
                        if (streaming || input.len() >= stream_floor())
                            && (output.as_mut_ptr() as usize).is_multiple_of(16)
                        {
                            // SAFETY: the streaming kernel requires a 16-byte
                            // aligned output, checked above. `streaming` asks
                            // for a one-shot write; otherwise the threshold
                            // keeps NT to buffers that exceed the LLC.
                            unsafe { avx2::encode_unroll8_split::<2>(input, output, self.alphabet) }
                        } else {
                            // SAFETY: AVX2 is available and the output holds the
                            // exact encoded length checked above.
                            unsafe { avx2::encode_unroll8_split::<0>(input, output, self.alphabet) }
                        }
                    } else {
                        unsafe { avx2::encode(input, output, self.alphabet) }
                    }
                }
            };
            out = start / 3 * 4;
        }
        #[cfg(target_arch = "aarch64")]
        if input.len() >= 24 && neon_available() {
            // SAFETY: runtime detection guarantees NEON and the exact output
            // length has been checked above.
            start = unsafe { neon::encode(input, output, self.alphabet) };
            out = start / 3 * 4;
        }
        // Rule 1: four 6-bit symbols are mapped in parallel inside a u32.
        // Adding a biased constant sets each byte's top bit iff its value
        // exceeds the relevant interval boundary; none of the byte sums
        // reaches 256, so there is no carry between lanes.
        let remaining = input.len() - start;
        let groups = remaining / 3;
        let table = match self.alphabet {
            Alphabet::Standard => STANDARD_ALPHABET,
            Alphabet::UrlSafe => URL_ALPHABET,
        };
        if remaining < 24 {
            // Rule 1: for a short tail, two 12-bit lookups map one 3-byte
            // group to four characters. The 8 KiB table stays in L1 and cuts
            // the lookup count in half compared with four 64-byte lookups.
            let pairs = match self.alphabet {
                Alphabet::Standard => &STANDARD_PAIRS,
                Alphabet::UrlSafe => &URL_PAIRS,
            };
            for i in 0..groups {
                let p = start + i * 3;
                let (a, b, c) = (input[p], input[p + 1], input[p + 2]);
                let first = pairs[((a as usize) << 4) | (b as usize >> 4)];
                let second = pairs[(((b & 15) as usize) << 8) | c as usize];
                let chars = ((first as u32) | ((second as u32) << 16)).to_le_bytes();
                // SAFETY: four bytes are reserved by the output length check.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        chars.as_ptr(),
                        output.as_mut_ptr().add(out).cast(),
                        4,
                    );
                }
                out += 4;
            }
        } else {
            for i in 0..groups {
                let p = start + i * 3;
                let (a, b, c) = (input[p], input[p + 1], input[p + 2]);
                let x = u32::from_le_bytes([
                    a >> 2,
                    (a & 3) << 4 | b >> 4,
                    (b & 15) << 2 | c >> 6,
                    c & 63,
                ]);
                let above = |bias: u32| ((x.wrapping_add(bias)) & 0x8080_8080) >> 7;
                let gt25 = above(0x6666_6666);
                let gt51 = above(0x4c4c_4c4c);
                let gt61 = above(0x4242_4242);
                let gt62 = above(0x4141_4141);
                let (sub, add) = match self.alphabet {
                    Alphabet::Standard => (15, 3),
                    Alphabet::UrlSafe => (13, 49),
                };
                let chars = x
                    .wrapping_add(0x4141_4141)
                    .wrapping_add(gt25 * 6)
                    .wrapping_sub(gt51 * 75)
                    .wrapping_sub(gt61 * sub)
                    .wrapping_add(gt62 * add);
                let chars = chars.to_le_bytes();
                // SAFETY: four bytes are reserved by the output length check.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        chars.as_ptr(),
                        output.as_mut_ptr().add(out).cast(),
                        4,
                    );
                }
                out += 4;
            }
        }
        // At most two bytes remain after the whole groups; encode them with
        // the same table and append the configured padding.
        let tail = &input[start + groups * 3..];
        if !tail.is_empty() {
            let a = tail[0];
            output[out].write(table[(a >> 2) as usize]);
            if tail.len() == 2 {
                let b = tail[1];
                output[out + 1].write(table[((a & 3) << 4 | b >> 4) as usize]);
                output[out + 2].write(table[((b & 15) << 2) as usize]);
                out += 3;
                if self.padding {
                    output[out].write(b'=');
                    out += 1;
                }
            } else {
                output[out + 1].write(table[((a & 3) << 4) as usize]);
                out += 2;
                if self.padding {
                    output[out].write(b'=');
                    output[out + 1].write(b'=');
                    out += 2;
                }
            }
        }
        Ok(out)
    }

    /// Append encoded input to an existing byte buffer.
    pub fn append_encode(self, input: &[u8], output: &mut Vec<u8>) -> Result<usize, Error> {
        let len = self.encoded_len(input.len()).ok_or(Error::InvalidLength)?;
        let start = output.len();
        start.checked_add(len).ok_or(Error::InvalidLength)?;
        output.reserve(len);
        let spare = &mut output.spare_capacity_mut()[..len];
        let written = self.encode_uninit(input, spare)?;
        // The encoder writes every byte of the exact encoded length.
        debug_assert_eq!(written, len);
        // SAFETY: `encode_uninit` initialized the first `written` bytes of the
        // spare capacity.
        unsafe { output.set_len(start + written) };
        Ok(written)
    }

    /// Encode input as an ASCII string.
    pub fn encode_to_string(self, input: &[u8]) -> Result<String, Error> {
        let mut out = Vec::new();
        self.append_encode(input, &mut out)?;
        // SAFETY: the encoder only emits the alphabet bytes and `=`, which
        // are all ASCII, so the buffer is valid UTF-8.
        Ok(unsafe { String::from_utf8_unchecked(out) })
    }
}

/// Encode a data URL using standard padded Base64. The MIME type is not validated.
pub fn encode_data_url(mime: &str, input: &[u8]) -> Result<String, Error> {
    let len = 5usize
        .checked_add(mime.len())
        .and_then(|n| n.checked_add(8))
        .and_then(|n| n.checked_add(STANDARD.encoded_len(input.len())?))
        .ok_or(Error::InvalidLength)?;
    let mut out = Vec::with_capacity(len);
    out.extend_from_slice(b"data:");
    out.extend_from_slice(mime.as_bytes());
    out.extend_from_slice(b";base64,");
    STANDARD.append_encode(input, &mut out)?;
    // SAFETY: `mime` is a `&str` and the prefix and Base64 payload are ASCII.
    Ok(unsafe { String::from_utf8_unchecked(out) })
}
