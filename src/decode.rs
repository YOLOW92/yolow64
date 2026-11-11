//! Base64 validation and decoding methods.

use std::mem::MaybeUninit;

#[cfg(target_arch = "x86_64")]
use crate::avx2;
#[cfg(target_arch = "x86_64")]
use crate::cpu::{avx2_available, bmi2_available};
#[cfg(target_arch = "aarch64")]
use crate::cpu::neon_available;
#[cfg(target_arch = "aarch64")]
use crate::neon;
use crate::tables::{STANDARD_DECODE_TABLE, URL_DECODE_TABLE};
use crate::{Alphabet, Codec, Error, maybe_uninit_slice};

impl Codec {
    #[inline]
    pub(crate) fn value(self, b: u8, offset: usize) -> Result<u8, Error> {
        match b {
            b'A'..=b'Z' => Ok(b - b'A'),
            b'a'..=b'z' => Ok(b - b'a' + 26),
            b'0'..=b'9' => Ok(b - b'0' + 52),
            b'+' if self.alphabet == Alphabet::Standard => Ok(62),
            b'/' if self.alphabet == Alphabet::Standard => Ok(63),
            b'-' if self.alphabet == Alphabet::UrlSafe => Ok(62),
            b'_' if self.alphabet == Alphabet::UrlSafe => Ok(63),
            _ => Err(Error::InvalidByte { byte: b, offset }),
        }
    }

    #[inline]
    pub(crate) fn whitespace(self, b: u8) -> bool {
        self.ignore_whitespace && matches!(b, b' ' | b'\t' | b'\r' | b'\n')
    }

    #[inline]
    pub(crate) fn allocation_len(self, input: &[u8]) -> Result<usize, Error> {
        if self.ignore_whitespace {
            return self.validate(input);
        }
        // Rule 2: for an unbroken base64 string, length and final padding
        // determine the exact result size. Validation still happens during
        // decoding; this calculation never treats guessed content as valid.
        let len = input.len();
        let tail = match len % 4 {
            0 => 0,
            1 => return Err(Error::InvalidLength),
            2 => 1,
            _ => 2,
        };
        let padding = if self.padding && len.is_multiple_of(4) {
            input
                .iter()
                .rev()
                .take(2)
                .take_while(|&&b| b == b'=')
                .count()
        } else {
            0
        };
        (len / 4 * 3 + tail)
            .checked_sub(padding)
            .ok_or(Error::InvalidPadding)
    }

    /// Validate input and return its exact decoded length.
    pub fn validate(self, input: &[u8]) -> Result<usize, Error> {
        let mut count = 0usize;
        let mut pads = 0usize;
        let mut last = 0u8;
        for (offset, &b) in input.iter().enumerate() {
            if self.whitespace(b) {
                continue;
            }
            if b == b'=' {
                if !self.padding {
                    return Err(Error::InvalidPadding);
                }
                pads += 1;
                if pads > 2 {
                    return Err(Error::InvalidPadding);
                }
            } else {
                if pads != 0 {
                    return Err(Error::InvalidPadding);
                }
                last = self.value(b, offset)?;
                count += 1;
            }
        }
        let rem = count % 4;
        if rem == 1 {
            return Err(Error::InvalidLength);
        }
        if self.padding {
            let expected = match rem {
                0 => 0,
                2 => 2,
                3 => 1,
                _ => unreachable!(),
            };
            if pads != expected {
                return Err(Error::InvalidPadding);
            }
        }
        if (rem == 2 && last & 15 != 0) || (rem == 3 && last & 3 != 0) {
            return Err(Error::InvalidPadding);
        }
        // 规律 2：完整 4 字符输出 3 字节；尾部长度精确可知。
        Ok(count / 4 * 3
            + match rem {
                2 => 1,
                3 => 2,
                _ => 0,
            })
    }

    /// Decode into a caller-provided buffer. On error it may contain a partial result.
    #[inline]
    pub fn decode(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        #[cfg(target_arch = "x86_64")]
        if self.padding && !self.ignore_whitespace && avx2_available() && input.len() >= 12 {
            // Exact no-padding short/block inputs share one compact dispatch.
            // This avoids the general whitespace, length and padding scan.
            let no_pad = if input.len() == 16 {
                !input[12..].contains(&b'=')
            } else {
                input.len().is_multiple_of(32) && !input[input.len() - 4..].contains(&b'=')
            };
            if no_pad {
                let len = input.len() / 4 * 3;
                if output.len() < len {
                    return Err(Error::OutputTooSmall);
                }
                let out = maybe_uninit_slice(output);
                let result = if input.len() == 16 {
                    let head: &[u8; 16] = input.try_into().unwrap();
                    // SAFETY: runtime detection guarantees AVX2 and the input
                    // is one complete 16-character block.
                    unsafe { avx2::decode16(head, &mut out[..12], self.alphabet) }.map(|()| 12)
                } else {
                    // SAFETY: runtime detection guarantees AVX2, the input is
                    // a whole number of 32-byte blocks, and the output prefix
                    // is the exact decoded length.
                    unsafe { avx2::decode(input, &mut out[..len], self.alphabet) }.map(|_| len)
                };
                return result.map_err(|offset| self.simd_error(input, offset));
            }
            if bmi2_available() {
                let table = match self.alphabet {
                    Alphabet::Standard => STANDARD_DECODE_TABLE.as_ptr(),
                    Alphabet::UrlSafe => URL_DECODE_TABLE.as_ptr(),
                };
                let out = maybe_uninit_slice(output);
                match input.len() {
                    12 if input[11] == b'=' && input[10] != b'=' => {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe { avx2::decode12(input, out, self.alphabet, table) };
                    }
                    24 if input[22..24] == *b"==" => {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe { avx2::decode24(input, out, self.alphabet, table) };
                    }
                    44 if input[43] == b'=' && input[42] != b'=' => {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe { avx2::decode44(input, out, self.alphabet, table) };
                    }
                    88 if input[86..88] == *b"==" => {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe { avx2::decode88(input, out, self.alphabet, table) };
                    }
                    n if n >= 172
                        && n % 128 == 44
                        && input[n - 1] == b'='
                        && input[n - 2] != b'=' =>
                    {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe {
                            avx2::decode_hash4_tail44(input, out, self.alphabet, table)
                        };
                    }
                    #[cfg(target_os = "linux")]
                    n if n >= 216 && n % 128 == 88 && input[n - 2..] == *b"==" => {
                        // SAFETY: AVX2 and BMI2 are available, and trailing
                        // padding was checked above.
                        return unsafe {
                            avx2::decode_hash4_tail88(input, out, self.alphabet, table)
                        };
                    }
                    _ => {}
                }
            }
        }
        self.decode_uninit(input, maybe_uninit_slice(output))
    }

    pub(crate) fn decode_uninit(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        // Small whitespace-bearing inputs would otherwise run a strict decode
        // over the raw prefix, fail at the first whitespace byte and then redo
        // the work after filtering. Short inputs go straight to the staging
        // path; longer ones probe the leading bytes first so whitespace-free
        // payloads still use the strict fast path.
        if self.ignore_whitespace && input.len() <= 512 {
            return self.decode_whitespace_short(input, output);
        }
        if self.ignore_whitespace
            && input.len() <= 4096
            && input[..input.len().min(256)]
                .iter()
                .any(|&byte| self.whitespace(byte))
        {
            return self.decode_whitespace(input, output);
        }
        #[cfg(target_arch = "x86_64")]
        if !self.ignore_whitespace
            && self.padding
            && input.len() >= 12
            && avx2_available()
            && bmi2_available()
        {
            let table = match self.alphabet {
                Alphabet::Standard => STANDARD_DECODE_TABLE.as_ptr(),
                Alphabet::UrlSafe => URL_DECODE_TABLE.as_ptr(),
            };
            match input.len() {
                12 if input[11] == b'=' && input[10] != b'=' => {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe { avx2::decode12(input, output, self.alphabet, table) };
                }
                24 if input[22..24] == *b"==" => {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe { avx2::decode24(input, output, self.alphabet, table) };
                }
                44 if input[43] == b'=' && input[42] != b'=' => {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe { avx2::decode44(input, output, self.alphabet, table) };
                }
                88 if input[86..88] == *b"==" => {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe { avx2::decode88(input, output, self.alphabet, table) };
                }
                n if n >= 172
                    && n % 128 == 44
                    && input[n - 1] == b'='
                    && input[n - 2] != b'=' =>
                {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe {
                        avx2::decode_hash4_tail44(input, output, self.alphabet, table)
                    };
                }
                #[cfg(target_os = "linux")]
                n if n >= 216 && n % 128 == 88 && input[n - 2..] == *b"==" => {
                    // SAFETY: AVX2 and BMI2 were detected above.
                    return unsafe {
                        avx2::decode_hash4_tail88(input, output, self.alphabet, table)
                    };
                }
                _ => {}
            }
        }
        #[cfg(target_arch = "x86_64")]
        if !self.ignore_whitespace && (input.len() == 16 || input.len() >= 32) && avx2_available() {
            return self.decode_strict_avx2(input, output);
        }
        #[cfg(target_arch = "x86_64")]
        if input.len() >= 32 && avx2_available() {
            // Rule 2: a block can include the final quartet only when it has
            // no padding. Otherwise leave that quartet to the scalar tail.
            let padded = input.len() >= 2 && input[input.len() - 2..].contains(&b'=');
            let tail = if padded { 4 } else { 0 };
            let bulk_len = input.len().saturating_sub(tail) / 32 * 32;
            if bulk_len < 32 {
                return if self.ignore_whitespace {
                    self.decode_whitespace(input, output)
                } else {
                    self.decode_short(input, output)
                };
            }
            // Decode the at most 35-byte tail once into a small scratch area.
            // This doubles as validation and avoids a separate length pass;
            // errors keep their original byte offsets.
            let mut scratch = [MaybeUninit::<u8>::uninit(); 32];
            let tail = if self.ignore_whitespace {
                self.decode_whitespace(&input[bulk_len..], &mut scratch)
            } else {
                self.decode_short(&input[bulk_len..], &mut scratch)
            };
            let tail_len = match tail {
                Ok(n) => n,
                // The raw chunk boundary can split a quartet once whitespace
                // shifts the character grid; redo the whole input in that case.
                Err(_) if self.ignore_whitespace => {
                    return self.decode_whitespace(input, output);
                }
                Err(Error::InvalidByte { byte, offset }) => {
                    return Err(Error::InvalidByte {
                        byte,
                        offset: offset + bulk_len,
                    });
                }
                Err(err) => return Err(err),
            };
            let prefix_len = bulk_len / 4 * 3;
            let len = prefix_len + tail_len;
            if output.len() < len {
                if self.ignore_whitespace {
                    return self.decode_whitespace(input, output);
                }
                return Err(Error::OutputTooSmall);
            }
            // SAFETY: runtime detection guarantees AVX2, the input prefix is a
            // whole number of 32-byte blocks and output has exact capacity.
            if let Err(offset) = unsafe { avx2::decode(&input[..bulk_len], output, self.alphabet) }
            {
                if self.ignore_whitespace {
                    return self.decode_whitespace(input, output);
                }
                return if input[offset] == b'=' {
                    Err(Error::InvalidPadding)
                } else {
                    Err(Error::InvalidByte {
                        byte: input[offset],
                        offset,
                    })
                };
            }
            // The capacity check above covers `len`; the scratch area holds
            // exactly the tail's decoded bytes.
            // SAFETY: the tail scratch is initialized for `tail_len` bytes and the
            // output region has exactly that many bytes.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    scratch.as_ptr(),
                    output.as_mut_ptr().add(prefix_len).cast(),
                    tail_len,
                );
            }
            return Ok(len);
        }
        #[cfg(target_arch = "aarch64")]
        if input.len() >= 64 && neon_available() {
            let bulk_len = input.len().saturating_sub(4) / 16 * 16;
            let tail = self.validate(&input[bulk_len..]).map_err(|err| match err {
                Error::InvalidByte { byte, offset } => Error::InvalidByte {
                    byte,
                    offset: offset + bulk_len,
                },
                other => other,
            });
            let tail_len = match tail {
                Ok(n) => n,
                Err(_) if self.ignore_whitespace => return self.decode_whitespace(input, output),
                Err(err) => return Err(err),
            };
            let prefix_len = bulk_len / 4 * 3;
            let len = prefix_len + tail_len;
            if output.len() < len {
                if self.ignore_whitespace {
                    return self.decode_whitespace(input, output);
                }
                return Err(Error::OutputTooSmall);
            }
            // SAFETY: runtime detection guarantees NEON, the input prefix has
            // whole 16-byte blocks and output has sufficient capacity.
            if let Err(offset) = unsafe { neon::decode(&input[..bulk_len], output, self.alphabet) }
            {
                if self.ignore_whitespace {
                    return self.decode_whitespace(input, output);
                }
                return if input[offset] == b'=' {
                    Err(Error::InvalidPadding)
                } else {
                    Err(Error::InvalidByte {
                        byte: input[offset],
                        offset,
                    })
                };
            }
            self.decode_uninit(&input[bulk_len..], &mut output[prefix_len..])?;
            return Ok(len);
        }
        if !self.ignore_whitespace {
            self.decode_short(input, output)
        } else {
            self.decode_scalar(input, output)
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[inline]
    fn simd_error(self, input: &[u8], offset: usize) -> Error {
        if input[offset] == b'=' {
            Error::InvalidPadding
        } else {
            Error::InvalidByte {
                byte: input[offset],
                offset,
            }
        }
    }

    /// Strict, whitespace-free AVX2 decode. Complete 32-byte blocks decode in
    /// bulk; when the input is not a multiple of 32, one overlapping block
    /// covers the final 32 characters so at most three characters plus the
    /// padded quartet remain scalar.
    #[cfg(target_arch = "x86_64")]
    fn decode_strict_avx2(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        // The strict allocation length is O(1): input length and the final
        // padding bytes determine it exactly. The bytes are still validated
        // during decoding.
        let len = self.allocation_len(input)?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }

        let valid_end = if self.padding
            && input.len().is_multiple_of(4)
            && input.len() >= 4
            && input[input.len() - 2..].contains(&b'=')
        {
            input.len() - 4
        } else {
            input.len()
        };
        let full_end = valid_end - valid_end % 4;
        let prefix_len = full_end / 4 * 3;

        if (32..=1800).contains(&full_end) {
            // SAFETY: runtime detection guarantees AVX2; `full_end` is a
            // multiple of four and its output prefix is exact.
            if let Err(offset) = unsafe {
                avx2::decode_any(&input[..full_end], &mut output[..prefix_len], self.alphabet)
            } {
                return Err(if input[offset] == b'=' {
                    Error::InvalidPadding
                } else {
                    Error::InvalidByte {
                        byte: input[offset],
                        offset,
                    }
                });
            }
            if full_end != input.len() {
                self.decode_short(&input[full_end..], &mut output[prefix_len..len])
                    .map_err(|err| match err {
                        Error::InvalidByte { byte, offset } => Error::InvalidByte {
                            byte,
                            offset: full_end + offset,
                        },
                        other => other,
                    })?;
            }
            return Ok(len);
        }

        if full_end >= 32 {
            let head_end = full_end / 32 * 32;
            // SAFETY: runtime detection guarantees AVX2; `head_end` is a whole
            // number of 32-byte blocks and its output prefix is exact.
            if let Err(offset) = unsafe {
                avx2::decode(
                    &input[..head_end],
                    &mut output[..head_end / 4 * 3],
                    self.alphabet,
                )
            } {
                return Err(if input[offset] == b'=' {
                    Error::InvalidPadding
                } else {
                    Error::InvalidByte {
                        byte: input[offset],
                        offset,
                    }
                });
            }
            let rem = full_end - head_end;
            if rem != 0 {
                // One overlapping block is cheaper than decoding up to seven
                // scalar quartets. It rewrites bytes already produced by the
                // head with identical values.
                let start = full_end - 32;
                let out_start = start / 4 * 3;
                // SAFETY: the overlapping block is exactly 32 alphabet bytes,
                // and its 24-byte output ends at `prefix_len`.
                if let Err(offset) = unsafe {
                    avx2::decode(
                        &input[start..full_end],
                        &mut output[out_start..],
                        self.alphabet,
                    )
                } {
                    let offset = start + offset;
                    return Err(if input[offset] == b'=' {
                        Error::InvalidPadding
                    } else {
                        Error::InvalidByte {
                            byte: input[offset],
                            offset,
                        }
                    });
                }
                if full_end != input.len() {
                    self.decode_short(&input[full_end..], &mut output[prefix_len..len])
                        .map_err(|err| match err {
                            Error::InvalidByte { byte, offset } => Error::InvalidByte {
                                byte,
                                offset: full_end + offset,
                            },
                            other => other,
                        })?;
                }
            } else {
                // Keep the remaining quarternary groups and the final
                // (possibly padded) quartet in one table-driven pass.
                let scalar = &input[head_end..];
                let out_start = head_end / 4 * 3;
                self.decode_short(scalar, &mut output[out_start..len])
                    .map_err(|err| match err {
                        Error::InvalidByte { byte, offset } => Error::InvalidByte {
                            byte,
                            offset: head_end + offset,
                        },
                        other => other,
                    })?;
            }
        } else if full_end == 16 {
            let head: &[u8; 16] = input[..16].try_into().unwrap();
            // SAFETY: runtime detection guarantees AVX2 and the first 16 bytes
            // form one complete 16-byte block.
            if let Err(offset) = unsafe { avx2::decode16(head, &mut output[..12], self.alphabet) } {
                return Err(if input[offset] == b'=' {
                    Error::InvalidPadding
                } else {
                    Error::InvalidByte {
                        byte: input[offset],
                        offset,
                    }
                });
            }
            if full_end != input.len() {
                self.decode_short(&input[full_end..], &mut output[prefix_len..len])
                    .map_err(|err| match err {
                        Error::InvalidByte { byte, offset } => Error::InvalidByte {
                            byte,
                            offset: full_end + offset,
                        },
                        other => other,
                    })?;
            }
        } else {
            self.decode_short(input, &mut output[..len])?;
        }
        Ok(len)
    }

    pub(crate) fn decode_short(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        // Rule 1: one table lookup per byte replaces the range-match chain for
        // short, whitespace-free input. Anything unusual (including malformed
        // padding) falls back so error variants and offsets stay identical.
        let table = match self.alphabet {
            Alphabet::Standard => &STANDARD_DECODE_TABLE,
            Alphabet::UrlSafe => &URL_DECODE_TABLE,
        };
        let mut end = input.len();
        let mut pads = 0usize;
        while end > 0 && input[end - 1] == b'=' {
            pads += 1;
            end -= 1;
            if pads > 2 {
                return self.decode_short_slow(input, output);
            }
        }
        if (pads != 0 && !self.padding) || end % 4 == 1 {
            return self.decode_short_slow(input, output);
        }
        if self.padding {
            let expected = match end % 4 {
                0 => 0,
                2 => 2,
                3 => 1,
                _ => return self.decode_short_slow(input, output),
            };
            if pads != expected {
                return self.decode_short_slow(input, output);
            }
        }
        let rem = end % 4;
        let len = end / 4 * 3
            + match rem {
                2 => 1,
                3 => 2,
                _ => 0,
            };
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        // Rule 1: four lookups produce 24 bits; the three significant bytes
        // are written big-endian. Validation is fused into this loop. A byte
        // outside the alphabet produces 0xff, so one OR detects it; malformed
        // input then takes the slow path to keep exact variants and offsets.
        let mut at = 0;
        let mut out = 0;
        while at + 4 <= end {
            let a = table[input[at] as usize];
            let b = table[input[at + 1] as usize];
            let c = table[input[at + 2] as usize];
            let d = table[input[at + 3] as usize];
            if (a | b | c | d) == 0xff {
                return self.decode_short_slow(input, output);
            }
            let (a, b, c, d) = (u32::from(a), u32::from(b), u32::from(c), u32::from(d));
            let bits = a << 18 | b << 12 | c << 6 | d;
            output[out].write((bits >> 16) as u8);
            output[out + 1].write((bits >> 8) as u8);
            output[out + 2].write(bits as u8);
            at += 4;
            out += 3;
        }
        if rem == 2 {
            let a = table[input[at] as usize];
            let b = table[input[at + 1] as usize];
            if (a | b) == 0xff || b & 15 != 0 {
                return self.decode_short_slow(input, output);
            }
            output[out].write((u32::from(a) << 2 | u32::from(b) >> 4) as u8);
        } else if rem == 3 {
            let a = table[input[at] as usize];
            let b = table[input[at + 1] as usize];
            let c = table[input[at + 2] as usize];
            if (a | b | c) == 0xff || c & 3 != 0 {
                return self.decode_short_slow(input, output);
            }
            output[out].write((u32::from(a) << 2 | u32::from(b) >> 4) as u8);
            output[out + 1].write((u32::from(b) << 4 | u32::from(c) >> 2) as u8);
        }
        Ok(len)
    }

    /// Careful path for malformed input: preserves the exact error variant
    /// and original byte offset.
    fn decode_short_slow(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        let prefix = input.len().saturating_sub(1) / 4 * 4;
        let tail_len = self.validate(&input[prefix..]).map_err(|e| match e {
            Error::InvalidByte { byte, offset } => Error::InvalidByte {
                byte,
                offset: prefix + offset,
            },
            other => other,
        })?;
        let len = prefix / 4 * 3 + tail_len;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        for (i, chunk) in input[..prefix].as_chunks::<4>().0.iter().enumerate() {
            let p = i * 4;
            let a = self.value(chunk[0], p)?;
            let b = self.value(chunk[1], p + 1)?;
            let c = self.value(chunk[2], p + 2)?;
            let d = self.value(chunk[3], p + 3)?;
            let o = i * 3;
            output[o].write(a << 2 | b >> 4);
            output[o + 1].write(b << 4 | c >> 2);
            output[o + 2].write(c << 6 | d);
        }
        self.decode_scalar(&input[prefix..], &mut output[prefix / 4 * 3..])?;
        Ok(len)
    }

    fn decode_whitespace_short(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        // A common short MIME value has only trailing CRLF. Avoid copying its
        // payload; any interior whitespace or error falls back to filtering.
        if input.len() < 64 && input.last().is_some_and(|&b| self.whitespace(b)) {
            let mut end = input.len();
            while end > 0 && self.whitespace(input[end - 1]) {
                end -= 1;
            }
            let strict = Codec {
                ignore_whitespace: false,
                ..self
            };
            if let Ok(n) = strict.decode_uninit(&input[..end], output) {
                return Ok(n);
            }
        }
        let mut buf = [MaybeUninit::<u8>::uninit(); 512];
        #[cfg(target_arch = "x86_64")]
        let filtered = if input.len() >= 64 && avx2_available() {
            // SAFETY: runtime detection guarantees AVX2, and the staging
            // buffer is large enough for the entire input.
            Some(unsafe { avx2::filter_whitespace(input, &mut buf) }.1)
        } else {
            None
        };
        #[cfg(not(target_arch = "x86_64"))]
        let filtered: Option<usize> = None;
        let used = filtered.unwrap_or_else(|| {
            let mut used = 0;
            for &byte in input {
                if !self.whitespace(byte) {
                    buf[used].write(byte);
                    used += 1;
                }
            }
            used
        });
        // SAFETY: every element in the prefix was initialized by the loop;
        // the caller restricts input length to the size of the buffer.
        let filled = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), used) };
        let strict = Codec {
            ignore_whitespace: false,
            ..self
        };
        match strict.decode_uninit(filled, output) {
            Ok(n) => Ok(n),
            Err(_) => self.decode_scalar(input, output),
        }
    }

    /// Fast path for the dominant MIME layout: 76 base64 characters followed
    /// by CRLF on every line. The structure is checked without touching the
    /// payload, then lines are copied in batches and decoded strictly.
    fn decode_mime_76(self, input: &[u8], output: &mut [MaybeUninit<u8>]) -> Option<usize> {
        const LINES: usize = 64;
        const STAGE_LEN: usize = 76 * LINES;
        fn is_space(b: u8) -> bool {
            matches!(b, b' ' | b'\t' | b'\r' | b'\n')
        }

        if input.len() < 78 {
            return None;
        }
        let mut at = 0usize;
        let mut chars = 0usize;
        while at + 78 <= input.len() {
            if input[at + 76] == b'\r' && input[at + 77] == b'\n' {
                at += 78;
            } else {
                break;
            }
        }
        let mut i = at;
        while i < input.len() && !is_space(input[i]) {
            chars += 1;
            i += 1;
        }
        if chars > 76 || !input[i..].iter().all(|&b| is_space(b)) {
            return None;
        }

        #[repr(align(64))]
        struct Stage {
            bytes: [MaybeUninit<u8>; STAGE_LEN],
        }
        let mut stage = Stage {
            bytes: [MaybeUninit::uninit(); STAGE_LEN],
        };
        let strict = Codec {
            ignore_whitespace: false,
            ..self
        };
        let mut read = 0usize;
        let mut written = 0usize;
        loop {
            let mut used = 0usize;
            while used < STAGE_LEN
                && read + 78 <= input.len()
                && input[read + 76] == b'\r'
                && input[read + 77] == b'\n'
            {
                // SAFETY: both ranges are within their allocations; the stage
                // has room for a whole 76-byte line and the input line is
                // fully in bounds.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        input.as_ptr().add(read),
                        stage.bytes.as_mut_ptr().add(used).cast::<u8>(),
                        76,
                    );
                }
                read += 78;
                used += 76;
            }
            if used == 0 {
                break;
            }
            // SAFETY: the loop above initialized exactly `used` bytes.
            let filled =
                unsafe { std::slice::from_raw_parts(stage.bytes.as_ptr().cast::<u8>(), used) };
            let n = strict.decode_uninit(filled, &mut output[written..]).ok()?;
            written += n;
        }

        // The final line may be shorter than 76 characters and may include
        // padding. It is guaranteed by the structural check above to fit.
        let mut used = 0usize;
        while read < input.len() {
            let b = input[read];
            if !is_space(b) {
                stage.bytes[used].write(b);
                used += 1;
            }
            read += 1;
        }
        if used != 0 {
            // SAFETY: the loop above initialized exactly `used` bytes.
            let filled =
                unsafe { std::slice::from_raw_parts(stage.bytes.as_ptr().cast::<u8>(), used) };
            let n = strict.decode_uninit(filled, &mut output[written..]).ok()?;
            written += n;
        }
        Some(written)
    }

    /// Strip whitespace in bounded blocks; invalid input is rechecked against
    /// the original bytes to preserve the original error offsets.
    fn decode_whitespace(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        if input.len() >= 256
            && let Some(n) = self.decode_mime_76(input, output)
        {
            return Ok(n);
        }
        // Rule 4: a 4 KiB staging buffer stays comfortably within L1 cache.
        // Its performance is benchmarked against the existing codec plus a
        // reused filtering buffer; the constant is still provisional.
        // The buffer is uninitialized on purpose: only the written prefix is
        // ever read, and zeroing 4 KiB per call showed up as a large fixed
        // cost for short inputs in the disassembly. The 64-byte alignment
        // keeps the SIMD decoder reading whole cache lines.
        #[repr(align(64))]
        struct Staging {
            bytes: [MaybeUninit<u8>; 4096],
        }
        let mut staging = Staging {
            bytes: [MaybeUninit::<u8>::uninit(); 4096],
        };
        let buf = &mut staging.bytes;
        let (mut used, mut out) = (0usize, 0usize);
        let strict = Codec {
            ignore_whitespace: false,
            ..self
        };
        let mut remaining = input;
        while !remaining.is_empty() {
            #[cfg(target_arch = "x86_64")]
            let filtered = if remaining.len() >= 256 && avx2_available() {
                // SAFETY: runtime detection guarantees AVX2; the filter writes
                // at most the supplied space and initializes its output prefix.
                Some(unsafe { avx2::filter_whitespace(remaining, &mut buf[used..]) })
            } else {
                None
            };
            #[cfg(not(target_arch = "x86_64"))]
            let filtered: Option<(usize, usize)> = None;
            let (consumed, added) = filtered.unwrap_or_else(|| {
                let count = remaining.len().min(buf.len() - used);
                let mut added = 0;
                for &byte in &remaining[..count] {
                    if !self.whitespace(byte) {
                        buf[used + added].write(byte);
                        added += 1;
                    }
                }
                (count, added)
            });
            remaining = &remaining[consumed..];
            used += added;
            if used == buf.len() {
                // Keep the last quartet: padding is only valid at the end.
                let prefix = buf.len() - 4;
                let need = prefix / 4 * 3;
                if output.len() - out < need {
                    return self.decode_scalar(input, output);
                }
                // SAFETY: the loop wrote buf[0..used] exactly, and used is
                // the full length here, so the whole prefix is initialized.
                let filled =
                    unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), prefix) };
                match strict.decode_uninit(filled, &mut output[out..]) {
                    Ok(n) => out += n,
                    Err(_) => return self.decode_scalar(input, output),
                }
                buf.copy_within(prefix.., 0);
                used = 4;
            }
        }
        // SAFETY: the loop wrote buf[0..used] exactly.
        let filled = unsafe { std::slice::from_raw_parts(buf.as_ptr().cast::<u8>(), used) };
        match strict.decode_uninit(filled, &mut output[out..]) {
            Ok(n) => Ok(out + n),
            Err(_) => self.decode_scalar(input, output),
        }
    }

    pub(crate) fn decode_scalar(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        let len = self.validate(input)?;
        if output.len() < len {
            return Err(Error::OutputTooSmall);
        }
        let (mut acc, mut bits, mut out) = (0u32, 0u32, 0usize);
        for (offset, &b) in input.iter().enumerate() {
            if self.whitespace(b) {
                continue;
            }
            if b == b'=' {
                break;
            }
            acc = (acc << 6) | u32::from(self.value(b, offset)?);
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                output[out].write((acc >> bits) as u8);
                out += 1;
                acc &= (1 << bits) - 1;
            }
        }
        Ok(out)
    }

    /// Decode to a new buffer.
    pub fn decode_to_vec(self, input: &[u8]) -> Result<Vec<u8>, Error> {
        // Raw input length gives a safe upper bound even when whitespace is
        // ignored. Decoding validates the bytes, so no separate length pass is
        // needed (that pass was a full extra scan for whitespace input).
        let len = input.len() / 4 * 3 + 2;
        let mut out = Vec::with_capacity(len);
        let written = self.decode_uninit(input, out.spare_capacity_mut())?;
        // SAFETY: decoding initialized exactly `written` bytes of spare
        // capacity.
        unsafe { out.set_len(written) };
        Ok(out)
    }

    /// Append decoded input to an existing buffer.
    pub fn append_decode(self, input: &[u8], output: &mut Vec<u8>) -> Result<usize, Error> {
        // Upper bound; see `decode_to_vec`. Decoding performs the validation.
        let len = input.len() / 4 * 3 + 2;
        let start = output.len();
        start.checked_add(len).ok_or(Error::InvalidLength)?;
        output.reserve(len);
        match self.decode_uninit(input, output.spare_capacity_mut()) {
            Ok(written) => {
                // SAFETY: decoding initialized exactly `written` bytes of
                // spare capacity.
                unsafe { output.set_len(start + written) };
                Ok(written)
            }
            Err(err) => Err(err),
        }
    }
}
