//! Incremental streaming Base64 decoder.

#[cfg(target_arch = "x86_64")]
use crate::avx2;
#[cfg(target_arch = "x86_64")]
use crate::cpu::avx2_available;
use crate::{Codec, Error};

#[inline]
fn scalar_first_special(input: &[u8], whitespace: bool) -> Option<usize> {
    input
        .iter()
        .position(|&b| b == b'=' || (whitespace && matches!(b, b' ' | b'\t' | b'\r' | b'\n')))
}

/// Incremental decoder. Output produced before an error is not rolled back;
/// callers must discard it if the complete message must be valid.
///
/// Offsets in `InvalidByte` refer to the original input across all chunks,
/// including ignored whitespace. After any error the decoder cannot be reused.
pub struct Decoder {
    codec: Codec,
    group: [u8; 4],
    used: usize,
    offset: usize,
    ended: bool,
    failed: bool,
}

impl Decoder {
    /// Create a decoder with the same alphabet, padding and whitespace rules
    /// as the non-streaming codec.
    pub fn new(codec: Codec) -> Self {
        Self {
            codec,
            group: [0; 4],
            used: 0,
            offset: 0,
            ended: false,
            failed: false,
        }
    }

    /// Consume a chunk, appending decoded complete groups to `output`.
    pub fn feed(&mut self, input: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidLength);
        }
        let Some(end) = self.offset.checked_add(input.len()) else {
            self.failed = true;
            return Err(Error::InvalidLength);
        };

        // Bytes after the padded quartet are only allowed to be whitespace.
        // This path is rare and small, so keep the byte-wise checks.
        if self.ended {
            self.feed_bytes(input, self.offset, output)?;
            self.offset = end;
            return Ok(());
        }

        // Complete a group left over from the previous chunk before entering
        // the bulk path.
        let mut at = 0usize;
        while self.used != 0 && at < input.len() {
            let byte = input[at];
            if self.codec.whitespace(byte) {
                at += 1;
                continue;
            }
            let offset = self.offset + at;
            if byte != b'='
                && let Err(err) = self.codec.value(byte, offset)
            {
                self.failed = true;
                return Err(err);
            }
            self.group[self.used] = byte;
            self.used += 1;
            at += 1;
            if self.used == 4 {
                let mut decoded = [0; 3];
                match self.codec.decode(&self.group, &mut decoded) {
                    Ok(n) => output.extend_from_slice(&decoded[..n]),
                    Err(err) => {
                        self.failed = true;
                        return Err(err);
                    }
                }
                self.ended = self.group.contains(&b'=');
                self.used = 0;
            }
        }
        if at == input.len() {
            self.offset = end;
            return Ok(());
        }
        if self.ended {
            self.feed_bytes(&input[at..], self.offset + at, output)?;
            self.offset = end;
            return Ok(());
        }

        let rest = &input[at..];
        let base = self.offset + at;
        let special = {
            #[cfg(target_arch = "x86_64")]
            {
                if avx2_available() {
                    // SAFETY: runtime detection guarantees AVX2.
                    unsafe { avx2::first_special(rest, self.codec.ignore_whitespace) }
                } else {
                    scalar_first_special(rest, self.codec.ignore_whitespace)
                }
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                scalar_first_special(rest, self.codec.ignore_whitespace)
            }
        };

        match special {
            None => {
                let bulk = rest.len() / 4 * 4;
                if bulk != 0 {
                    self.decode_bulk(&rest[..bulk], base, output)?;
                }
                for (i, &byte) in rest[bulk..].iter().enumerate() {
                    let offset = base + bulk + i;
                    if let Err(err) = self.codec.value(byte, offset) {
                        self.failed = true;
                        return Err(err);
                    }
                    self.group[self.used] = byte;
                    self.used += 1;
                }
            }
            Some(pos) if !self.codec.ignore_whitespace => {
                // The first `=` can only be padding in the final quartet.
                // Decode the complete groups before that quartet in bulk, then
                // let the byte-wise path validate the padded remainder.
                let bulk = pos / 4 * 4;
                if bulk != 0 {
                    self.decode_bulk(&rest[..bulk], base, output)?;
                }
                self.feed_bytes(&rest[bulk..], base + bulk, output)?;
            }
            Some(_) => {
                // Whitespace changes the character grid; preserving exact
                // streaming offsets needs the byte-wise path for this chunk.
                self.feed_bytes(rest, base, output)?;
            }
        }

        self.offset = end;
        Ok(())
    }

    fn decode_bulk(
        &mut self,
        input: &[u8],
        base: usize,
        output: &mut Vec<u8>,
    ) -> Result<(), Error> {
        debug_assert!(input.len().is_multiple_of(4));
        let old = output.len();
        let cap = input.len() / 4 * 3;
        output.reserve(cap);
        match self.codec.decode_uninit(input, output.spare_capacity_mut()) {
            Ok(n) => {
                // SAFETY: decoding initialized exactly `n` bytes of spare
                // capacity.
                unsafe { output.set_len(old + n) };
                Ok(())
            }
            Err(err) => {
                self.failed = true;
                Err(match err {
                    Error::InvalidByte { byte, offset } => Error::InvalidByte {
                        byte,
                        offset: base + offset,
                    },
                    other => other,
                })
            }
        }
    }

    fn feed_bytes(&mut self, input: &[u8], base: usize, output: &mut Vec<u8>) -> Result<(), Error> {
        for (i, &byte) in input.iter().enumerate() {
            if self.codec.whitespace(byte) {
                continue;
            }
            let offset = base + i;
            if self.ended {
                self.failed = true;
                return Err(Error::InvalidPadding);
            }
            if byte != b'='
                && let Err(err) = self.codec.value(byte, offset)
            {
                self.failed = true;
                return Err(err);
            }
            self.group[self.used] = byte;
            self.used += 1;
            if self.used == 4 {
                let mut decoded = [0; 3];
                match self.codec.decode(&self.group, &mut decoded) {
                    Ok(n) => output.extend_from_slice(&decoded[..n]),
                    Err(err) => {
                        self.failed = true;
                        return Err(err);
                    }
                }
                self.ended = self.group.contains(&b'=');
                self.used = 0;
            }
        }
        Ok(())
    }

    /// Finish the message, validating its final group and padding.
    pub fn finish(mut self, output: &mut Vec<u8>) -> Result<(), Error> {
        if self.failed {
            return Err(Error::InvalidLength);
        }
        if self.used != 0 {
            let mut decoded = [0; 3];
            let n = self.codec.decode(&self.group[..self.used], &mut decoded)?;
            output.extend_from_slice(&decoded[..n]);
            self.used = 0;
        }
        Ok(())
    }
}
