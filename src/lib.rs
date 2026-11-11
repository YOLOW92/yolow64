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
#[cfg(test)]
pub(crate) static ENCODE_VARIANT: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(test)]
pub(crate) static ENCODE_PREFETCH_DISTANCE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(8192);
#[cfg(test)]
pub(crate) static ENCODE_PREFETCH_NTA: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
#[cfg(test)]
pub(crate) static ENCODE_PREFETCH_WIDE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

mod compat;
pub use compat::decode_standard as decode_standard_compat;
#[cfg(feature = "parallel")]
mod parallel;
#[cfg(test)]
mod threshold_tests;

#[cfg(target_arch = "x86_64")]
mod avx2;
#[cfg(target_arch = "aarch64")]
mod neon;

#[cfg(target_arch = "x86_64")]
fn avx2_available() -> bool {
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::is_x86_feature_detected!("avx2"))
}

#[cfg(target_arch = "x86_64")]
fn bmi2_available() -> bool {
    #[cfg(test)]
    if FORCE_NO_BMI2.with(|forced| forced.get()) {
        return false;
    }
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::is_x86_feature_detected!("bmi2"))
}

#[cfg(all(test, target_arch = "x86_64"))]
thread_local! {
    /// Test-only switch used to exercise the no-BMI2 fallback on a CPU that
    /// has BMI2. Thread-local so it cannot affect parallel tests.
    pub(crate) static FORCE_NO_BMI2: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Smallest and largest cache sizes believed from CPUID. A value outside this
/// range is treated as unknown; an unknown LLC disables streaming stores.
#[cfg(target_arch = "x86_64")]
const MIN_CACHE_BYTES: usize = 64 * 1024;
#[cfg(target_arch = "x86_64")]
const MAX_CACHE_BYTES: usize = 2 * 1024 * 1024 * 1024;

/// Parse one deterministic-cache-parameters entry. Intel leaf 4 and AMD leaf
/// 0x8000_001d share this layout.
#[cfg(target_arch = "x86_64")]
fn parse_cache_leaf(regs: std::arch::x86_64::CpuidResult) -> Option<(u8, usize)> {
    let cache_type = regs.eax & 0x1f;
    if !matches!(cache_type, 1 | 3) {
        return None;
    }
    let level = ((regs.eax >> 5) & 0x7) as u8;
    if level < 2 {
        return None;
    }
    let line = (regs.ebx & 0xfff) as usize + 1;
    let partitions = ((regs.ebx >> 12) & 0x3ff) as usize + 1;
    let ways = ((regs.ebx >> 22) & 0x3ff) as usize + 1;
    let sets = regs.ecx as usize + 1;
    let size = line
        .checked_mul(partitions)?
        .checked_mul(ways)?
        .checked_mul(sets)?;
    if !(MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&size) {
        return None;
    }
    Some((level, size))
}

/// Largest data/unified cache reported by a deterministic-cache leaf.
#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
fn cache_leaf_size(leaf: u32) -> Option<usize> {
    let mut best: Option<(u8, usize)> = None;
    for index in 0..16 {
        // Leaf/index are documented architectural inputs and unknown leaves
        // return zeros.
        let regs = unsafe { std::arch::x86_64::__cpuid_count(leaf, index) };
        if regs.eax & 0x1f == 0 {
            break;
        }
        if let Some(entry) = parse_cache_leaf(regs)
            && best.is_none_or(|(level, _)| entry.0 > level)
        {
            best = Some(entry);
        }
    }
    best.map(|(_, size)| size)
}

/// Last-level cache size in bytes, if CPUID reports a plausible value.
#[cfg(target_arch = "x86_64")]
#[allow(unused_unsafe)]
pub(crate) fn llc_bytes() -> Option<usize> {
    let max_basic = unsafe { std::arch::x86_64::__cpuid(0) }.eax;
    if max_basic >= 4
        && let Some(size) = cache_leaf_size(4)
    {
        return Some(size);
    }
    let max_extended = unsafe { std::arch::x86_64::__cpuid(0x8000_0000) }.eax;
    if max_extended >= 0x8000_001d
        && let Some(size) = cache_leaf_size(0x8000_001d)
    {
        return Some(size);
    }
    if max_extended >= 0x8000_0006 {
        let regs = unsafe { std::arch::x86_64::__cpuid(0x8000_0006) };
        let l3 = ((regs.ecx >> 18) & 0x3fff) as usize * 512 * 1024;
        if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&l3) {
            return Some(l3);
        }
        // Some older AMD parts report only L2 here: bits 31:16 in 256 KiB
        // units. L2 is then the last-level cache for the streaming decision.
        let l2 = ((regs.ecx >> 16) & 0xffff) as usize * 256 * 1024;
        if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&l2) {
            return Some(l2);
        }
    }
    None
}

/// Input size past which the input plus encoded output no longer fits in the
/// last-level cache. The encoded output is 4/3 of the input, so the working set
/// is 7/3 of the input; `N >= 3/7 * LLC` is the crossover. In that regime
/// cached stores pay for a read-for-ownership of the output, while streaming
/// stores write it once.
#[cfg(target_arch = "x86_64")]
fn stream_floor_for(llc: Option<usize>) -> usize {
    match llc {
        Some(bytes) if (MIN_CACHE_BYTES..=MAX_CACHE_BYTES).contains(&bytes) => {
            (bytes / 7 * 3).max(MIN_CACHE_BYTES)
        }
        _ => usize::MAX,
    }
}

/// Cached stream floor for the current processor.
#[cfg(target_arch = "x86_64")]
fn stream_floor() -> usize {
    static FLOOR: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *FLOOR.get_or_init(|| stream_floor_for(llc_bytes()))
}

#[cfg(all(test, target_arch = "x86_64"))]
mod cache_tests {
    use super::{parse_cache_leaf, stream_floor_for};
    use std::arch::x86_64::CpuidResult;

    fn entry(level: u8, cache_type: u32, line: u32, ways: u32, sets: u32) -> CpuidResult {
        CpuidResult {
            eax: cache_type | (level as u32) << 5,
            ebx: (line - 1) | (ways - 1) << 22,
            ecx: sets - 1,
            edx: 0,
        }
    }

    #[test]
    fn parses_intel_and_amd_l3_shapes() {
        let intel = entry(3, 3, 64, 16, 16 * 1024);
        assert_eq!(parse_cache_leaf(intel), Some((3, 16 * 1024 * 1024)));
        let amd = entry(3, 3, 64, 16, 32 * 1024);
        assert_eq!(parse_cache_leaf(amd), Some((3, 32 * 1024 * 1024)));
        let l2 = entry(2, 3, 64, 8, 1024);
        assert_eq!(parse_cache_leaf(l2), Some((2, 512 * 1024)));
    }

    #[test]
    fn rejects_instruction_and_null_entries() {
        assert_eq!(parse_cache_leaf(entry(3, 0, 64, 16, 1024)), None);
        assert_eq!(parse_cache_leaf(entry(3, 2, 64, 16, 1024)), None);
    }

    #[test]
    fn rejects_implausible_sizes() {
        assert_eq!(parse_cache_leaf(entry(3, 3, 64, 16, 1)), None);
        assert_eq!(parse_cache_leaf(entry(3, 3, 64, 1024, u32::MAX)), None);
    }

    #[test]
    fn detected_cache_is_sane() {
        let llc = super::llc_bytes();
        let floor = super::stream_floor();
        println!("llc_bytes={llc:?} stream_floor={floor}");
        assert!(llc.is_none_or(|bytes| {
            (super::MIN_CACHE_BYTES..=super::MAX_CACHE_BYTES).contains(&bytes)
        }));
        assert!(floor == usize::MAX || floor >= super::MIN_CACHE_BYTES);
    }

    #[test]
    fn floor_is_three_sevenths_of_llc() {
        assert_eq!(stream_floor_for(Some(16 * 1024 * 1024)), 7_190_235);
        assert_eq!(stream_floor_for(Some(512 * 1024)), 224_694);
        assert_eq!(stream_floor_for(Some(64 * 1024)), 65_536);
        assert_eq!(stream_floor_for(None), usize::MAX);
        assert_eq!(stream_floor_for(Some(0)), usize::MAX);
    }
}

#[cfg(target_arch = "aarch64")]
fn neon_available() -> bool {
    static DETECTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *DETECTED.get_or_init(|| std::arch::is_aarch64_feature_detected!("neon"))
}

/// Byte-to-sextet table for the strict scalar path; 0xff marks a non-alphabet
/// byte. Two 256-byte tables stay in L1 and remove the range-match chain for
/// short inputs.
const STANDARD_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
const URL_ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

const fn decode_table(alphabet: &[u8; 64]) -> [u8; 256] {
    let mut table = [0xff; 256];
    let mut i = 0;
    while i < 64 {
        table[alphabet[i] as usize] = i as u8;
        i += 1;
    }
    table
}

static STANDARD_DECODE_TABLE: [u8; 256] = decode_table(STANDARD_ALPHABET);
static URL_DECODE_TABLE: [u8; 256] = decode_table(URL_ALPHABET);

/// Decode eight base64 characters to a 48-bit value ordered v0..v7 from high
/// to low.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "bmi2")]
unsafe fn pext8(input: *const u8, table: *const u8) -> u64 {
    let x = (input as *const u64).read_unaligned();
    // SAFETY: `table` is a 256-byte decode table; the shifts select one byte.
    let y = ((*table.add((x & 255) as usize) as u64) << 56)
        | ((*table.add(((x >> 8) & 255) as usize) as u64) << 48)
        | ((*table.add(((x >> 16) & 255) as usize) as u64) << 40)
        | ((*table.add(((x >> 24) & 255) as usize) as u64) << 32)
        | ((*table.add(((x >> 32) & 255) as usize) as u64) << 24)
        | ((*table.add(((x >> 40) & 255) as usize) as u64) << 16)
        | ((*table.add(((x >> 48) & 255) as usize) as u64) << 8)
        | (*table.add(((x >> 56) & 255) as usize) as u64);
    if y & 0x8080_8080_8080_8080 != 0 {
        return u64::MAX;
    }
    std::arch::x86_64::_pext_u64(y, 0x3f3f_3f3f_3f3f_3f3f)
}

/// Decode three base64 characters to an 18-bit value ordered v0..v2 from high
/// to low. The fourth character is padding and is not passed here.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "bmi2")]
unsafe fn pext3(input: *const u8, table: *const u8) -> u32 {
    let x = (input as *const u32).read_unaligned();
    // SAFETY: `table` is a 256-byte decode table; the shifts select one byte.
    let y = ((*table.add((x & 255) as usize) as u64) << 16)
        | ((*table.add(((x >> 8) & 255) as usize) as u64) << 8)
        | (*table.add(((x >> 16) & 255) as usize) as u64);
    if y & 0x0080_8080 != 0 {
        return u32::MAX;
    }
    std::arch::x86_64::_pext_u64(y, 0x3f_3f3f) as u32
}

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

static STANDARD_PAIRS: [u16; 4096] = encode_pair_table(STANDARD_ALPHABET);
static URL_PAIRS: [u16; 4096] = encode_pair_table(URL_ALPHABET);

/// View initialized bytes as `MaybeUninit` bytes. Every initialized `u8` is
/// a valid `MaybeUninit<u8>`; the reverse operation is what requires care.
fn maybe_uninit_slice(output: &mut [u8]) -> &mut [MaybeUninit<u8>] {
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

impl Codec {
    /// Exact output length, or `None` if it cannot fit in a `usize`.
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

    fn value(self, b: u8, offset: usize) -> Result<u8, Error> {
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

    /// Encode into a caller-provided buffer.
    ///
    /// This is the default in-memory policy: the output stays cacheable below
    /// the last-level-cache crossover, which is best when the caller reuses or
    /// rereads the buffer.
    pub fn encode(self, input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        #[cfg(target_arch = "x86_64")]
        if matches!(input.len(), 8 | 16 | 32) && self.padding && avx2_available() {
            #[cfg(test)]
            let short_variant = ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
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

    fn encode_uninit(self, input: &[u8], output: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
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

    fn encode_uninit_with(
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
            let short_variant = ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
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
            let variant = ENCODE_VARIANT.load(std::sync::atomic::Ordering::Relaxed);
            #[cfg(not(test))]
            let variant = 0u8;
            start = match variant {
                1 => unsafe { avx2::encode(input, output, self.alphabet) },
                #[cfg(test)]
                2 => unsafe { avx2::encode_unroll8_seq(input, output, self.alphabet) },
                #[cfg(test)]
                3 => unsafe { avx2::encode_countloop(input, output, self.alphabet) },
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

    fn whitespace(self, b: u8) -> bool {
        self.ignore_whitespace && matches!(b, b' ' | b'\t' | b'\r' | b'\n')
    }

    fn allocation_len(self, input: &[u8]) -> Result<usize, Error> {
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
        let padding = input
            .iter()
            .rev()
            .take(2)
            .take_while(|&&b| b == b'=')
            .count();
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
        if self.padding && !self.ignore_whitespace && avx2_available() && input.len() >= 16 {
            if input.len() == 24 && input[22..24] == *b"==" && bmi2_available() {
                let table = match self.alphabet {
                    Alphabet::Standard => STANDARD_DECODE_TABLE.as_ptr(),
                    Alphabet::UrlSafe => URL_DECODE_TABLE.as_ptr(),
                };
                // SAFETY: runtime detection guarantees AVX2 and BMI2, and the
                // trailing padding was checked above.
                return unsafe {
                    avx2::decode24(input, maybe_uninit_slice(output), self.alphabet, table)
                };
            }
            // Exact no-padding short inputs share one compact dispatch. This
            // avoids the general whitespace, length and padding scan.
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
            if input.len() == 172 && input[171] == b'=' && input[170] != b'=' && bmi2_available() {
                let table = match self.alphabet {
                    Alphabet::Standard => STANDARD_DECODE_TABLE.as_ptr(),
                    Alphabet::UrlSafe => URL_DECODE_TABLE.as_ptr(),
                };
                // SAFETY: runtime detection guarantees AVX2 and BMI2, and the
                // trailing padding was checked above.
                return unsafe {
                    avx2::decode172(input, maybe_uninit_slice(output), self.alphabet, table)
                };
            }
        }
        self.decode_uninit(input, maybe_uninit_slice(output))
    }

    fn decode_uninit(self, input: &[u8], output: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
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
            && input.len() == 44
            && input[43] == b'='
            && input[42] != b'='
            && avx2_available()
            && bmi2_available()
        {
            return self.decode_padded_44(input, output);
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
    #[inline(never)]
    fn decode_padded_44(
        self,
        input: &[u8],
        output: &mut [MaybeUninit<u8>],
    ) -> Result<usize, Error> {
        if output.len() < 32 {
            return Err(Error::OutputTooSmall);
        }
        // SAFETY: runtime detection guarantees AVX2 and the first 32
        // characters form one complete block.
        if let Err(offset) = unsafe { avx2::decode(&input[..32], &mut output[..24], self.alphabet) }
        {
            return Err(self.simd_error(input, offset));
        }
        let table = match self.alphabet {
            Alphabet::Standard => STANDARD_DECODE_TABLE.as_ptr(),
            Alphabet::UrlSafe => URL_DECODE_TABLE.as_ptr(),
        };
        // SAFETY: BMI2 was checked by the caller; the tail has twelve bytes.
        let z8 = unsafe { pext8(input.as_ptr().add(32), table) };
        let z3 = unsafe { pext3(input.as_ptr().add(40), table) };
        if z8 == u64::MAX || z3 == u32::MAX {
            return self.decode_strict_avx2(input, output);
        }
        if z3 & 0x03 != 0 {
            return Err(Error::InvalidPadding);
        }
        // SAFETY: the output has 32 bytes; the six new bytes start at 24 and
        // the final two at 30.
        unsafe {
            let p = output.as_mut_ptr().add(24).cast::<u8>();
            p.write((z8 >> 40) as u8);
            p.add(1).write((z8 >> 32) as u8);
            p.add(2).write((z8 >> 24) as u8);
            p.add(3).write((z8 >> 16) as u8);
            p.add(4).write((z8 >> 8) as u8);
            p.add(5).write(z8 as u8);
            p.add(6).write((z3 >> 10) as u8);
            p.add(7).write((z3 >> 2) as u8);
        }
        Ok(32)
    }

    #[cfg(target_arch = "x86_64")]
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

        let valid_end =
            if self.padding && input.len() >= 2 && input[input.len() - 2..].contains(&b'=') {
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

    fn decode_short(self, input: &[u8], output: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
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

    fn decode_scalar(self, input: &[u8], output: &mut [MaybeUninit<u8>]) -> Result<usize, Error> {
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
