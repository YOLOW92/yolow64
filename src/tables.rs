//! Lookup tables for scalar and short-input paths.

/// Byte-to-sextet table for the strict scalar path; 0xff marks a non-alphabet
/// byte. Two 256-byte tables stay in L1 and remove the range-match chain for
/// short inputs.
pub(crate) const STANDARD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
pub(crate) const URL_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

const fn decode_table(alphabet: &[u8; 64]) -> [u8; 256] {
    let mut table = [0xff; 256];
    let mut i = 0;
    while i < 64 {
        table[alphabet[i] as usize] = i as u8;
        i += 1;
    }
    table
}

pub(crate) static STANDARD_DECODE_TABLE: [u8; 256] = decode_table(STANDARD_ALPHABET);
pub(crate) static URL_DECODE_TABLE: [u8; 256] = decode_table(URL_ALPHABET);

/// Two output characters for a 12-bit input slab. The low byte is the first
/// character, so a little-endian u16 can be written directly.
const fn encode_pair_table(alphabet: &[u8; 64]) -> [u16; 4096] {
    let mut table = [0u16; 4096];
    let mut i = 0;
    while i < 4096 {
        let first = alphabet[i >> 6] as u16;
        let second = alphabet[i & 63] as u16;
        table[i] = first | (second << 8);
        i += 1;
    }
    table
}

pub(crate) static STANDARD_PAIRS: [u16; 4096] = encode_pair_table(STANDARD_ALPHABET);
pub(crate) static URL_PAIRS: [u16; 4096] = encode_pair_table(URL_ALPHABET);
