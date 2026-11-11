//! Base64 encoding and decoding, with runtime-dispatched AVX2 and
//! NEON kernels and scalar fallbacks.
//!
//! Whitespace mode ignores exactly ASCII space, tab, carriage return and line
//! feed. Padding is required when `padding` is true and forbidden otherwise.
//! Non-zero unused bits are rejected in both modes.
//!
//! The optional `parallel` feature adds cache-aware `encode_parallel` and
//! `decode_parallel` methods. The default APIs remain single-threaded.

use std::fmt;
use std::mem::MaybeUninit;

mod compat;
mod cpu;
mod decode;
mod encode;
#[cfg(feature = "parallel")]
mod parallel;
mod stream;
mod tables;
#[cfg(test)]
mod threshold_tests;

#[cfg(target_arch = "x86_64")]
mod avx2;
#[cfg(target_arch = "aarch64")]
mod neon;

pub use compat::decode_standard as decode_standard_compat;
pub use encode::encode_data_url;
pub use stream::Decoder;

/// View initialized bytes as `MaybeUninit` bytes. Every initialized `u8` is
/// a valid `MaybeUninit<u8>`; the reverse operation is what requires care.
#[inline]
pub(crate) fn maybe_uninit_slice(output: &mut [u8]) -> &mut [MaybeUninit<u8>] {
    // SAFETY: the two types have the same layout and alignment, and an
    // initialized byte is a valid `MaybeUninit<u8>`.
    unsafe { std::slice::from_raw_parts_mut(output.as_mut_ptr().cast(), output.len()) }
}

/// Alphabet used for encoding and decoding.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Alphabet {
    Standard,
    UrlSafe,
}

/// Codec options.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Codec {
    pub alphabet: Alphabet,
    pub padding: bool,
    pub ignore_whitespace: bool,
}

/// Standard alphabet, with mandatory padding and no whitespace.
pub const STANDARD: Codec = Codec {
    alphabet: Alphabet::Standard,
    padding: true,
    ignore_whitespace: false,
};

/// A malformed input or an insufficient output buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidByte { byte: u8, offset: usize },
    InvalidLength,
    InvalidPadding,
    OutputTooSmall,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidByte { byte, offset } => {
                write!(f, "Invalid byte {byte}, offset {offset}.")
            }
            Self::InvalidLength => f.write_str("Invalid input length."),
            Self::InvalidPadding => f.write_str("Invalid padding."),
            Self::OutputTooSmall => f.write_str("Output buffer too small."),
        }
    }
}

impl std::error::Error for Error {}
