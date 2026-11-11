use super::decode::{STANDARD_MEMBERSHIP, URL_MEMBERSHIP};
use super::encode::{ENCODE_PREFETCH, encode, encode_load16, mulhi_epu16};
use crate::Alphabet;
#[cfg(target_os = "linux")]
use std::arch::global_asm;
use std::arch::x86_64::*;

/// Test-only switch for selecting experimental AVX2 encode loop variants.
pub(crate) static ENCODE_VARIANT: std::sync::atomic::AtomicU8 =
    std::sync::atomic::AtomicU8::new(0);

/// Test-only prefetch distance for the split 8-block encoder.
pub(crate) static ENCODE_PREFETCH_DISTANCE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(4096);

/// Test-only hint selector for the split 8-block encoder (`true` = `NTA`).
pub(crate) static ENCODE_PREFETCH_NTA: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Test-only switch for prefetching all three input lines per 8-block step.
pub(crate) static ENCODE_PREFETCH_WIDE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(true);

/// Eight-block loop using the classic 16-byte load layout: low half at the
/// group start and high half at `start + 12`, with the same shuffle in both
/// 128-bit lanes. The high load reads four bytes past the 24-byte group; the
/// loop leaves at least a full block of slack and the remainder is handled by
/// the safe encoder.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_load16_turbo8(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let vh = _mm256_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10,
        9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! load_pair {
        ($off:expr) => {{
            // SAFETY: the loop leaves a full block of slack after the
            // 24-byte group, so both 16-byte loads fit.
            let lo = unsafe { _mm_loadu_si128(input.as_ptr().add($off).cast()) };
            let hi = unsafe { _mm_loadu_si128(input.as_ptr().add($off + 12).cast()) };
            _mm256_set_m128i(hi, lo)
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, vh);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    macro_rules! group4 {
        () => {{
            let b0 = load_pair!(at);
            let b1 = load_pair!(at + 24);
            let b2 = load_pair!(at + 48);
            let b3 = load_pair!(at + 72);
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            let r2 = convert!(b2);
            let r3 = convert!(b3);
            // SAFETY: four groups reserve four 32-byte stores.
            unsafe {
                _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
            }
            at += 96;
            out += 128;
        }};
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        group4!();
        group4!();
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Eight-block loop using two 16-byte loads per group and two-block phases.
/// This is the load pattern used by several classic SIMD encoders; keeping only
/// two groups live per phase avoids the pressure of loading all four first.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_load16_u8(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shuffle = _mm256_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10, 5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11,
        14, 13, 15, 14,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! load_pair {
        ($off:expr) => {{
            // SAFETY: the loop guarantees a complete 24-byte group.
            let lo = unsafe { _mm_loadu_si128(input.as_ptr().add($off).cast()) };
            let hi = unsafe { _mm_loadu_si128(input.as_ptr().add($off + 8).cast()) };
            _mm256_set_m128i(hi, lo)
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shuffle);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    macro_rules! phase {
        ($at:expr, $out:expr) => {{
            let b0 = load_pair!($at);
            let b1 = load_pair!($at + 24);
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            // SAFETY: the phase reserves two 32-byte stores.
            unsafe {
                _mm256_storeu_si256(output.as_mut_ptr().add($out).cast(), r0);
                _mm256_storeu_si256(output.as_mut_ptr().add($out + 32).cast(), r1);
            }
        }};
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        phase!(at, out);
        phase!(at + 48, out + 64);
        phase!(at + 96, out + 128);
        phase!(at + 144, out + 192);
        at += 192;
        out += 256;
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Eight-block loop with four two-block phases. Keeping only two blocks live
/// makes it easier for the scheduler to overlap loads, multiplies and stores.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_unroll8_2phase(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shifted);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    macro_rules! phase {
        ($at:expr, $out:expr) => {{
            // SAFETY: the loop guarantees the two 32-byte windows.
            let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add($at - 4).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add($at + 20).cast()) };
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            // SAFETY: the phase reserves two 32-byte stores.
            unsafe {
                _mm256_storeu_si256(output.as_mut_ptr().add($out).cast(), r0);
                _mm256_storeu_si256(output.as_mut_ptr().add($out + 32).cast(), r1);
            }
        }};
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        phase!(at, out);
        phase!(at + 48, out + 64);
        phase!(at + 96, out + 128);
        phase!(at + 144, out + 192);
        at += 192;
        out += 256;
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Four-block loop with two-block phases, like the classic SIMD encoder:
/// load and convert two blocks, store them, then load and convert the next two.
/// Keeping only two blocks live reduces register pressure compared with loading
/// all four first.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_load16_phased(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shuffle = _mm256_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10, 5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11,
        14, 13, 15, 14,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! load_pair {
        ($off:expr) => {{
            // SAFETY: the loop guarantees a complete 24-byte group.
            let lo = unsafe { _mm_loadu_si128(input.as_ptr().add($off).cast()) };
            let hi = unsafe { _mm_loadu_si128(input.as_ptr().add($off + 8).cast()) };
            _mm256_set_m128i(hi, lo)
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shuffle);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    while input.len() - at >= 96 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        let b0 = load_pair!(at);
        let b1 = load_pair!(at + 24);
        let r0 = convert!(b0);
        let r1 = convert!(b1);
        // SAFETY: four groups reserve four 32-byte stores; the first phase
        // writes the first two.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
        }
        let b2 = load_pair!(at + 48);
        let b3 = load_pair!(at + 72);
        let r2 = convert!(b2);
        let r3 = convert!(b3);
        // SAFETY: the second phase writes the remaining two groups.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
        }
        at += 96;
        out += 128;
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Eight-block unroll with compile-time load selection. After the first group,
/// eight blocks advance by 192 bytes, so `at & 63` is constant and the four
/// cache-line-crossing windows are known at compile time. Those windows use two
/// 16-byte loads; the others use one 32-byte load.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_unroll8_mixed(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! window {
        ($off:expr, $cross:expr) => {{
            let start = $off - 4;
            if $cross {
                // SAFETY: both 16-byte halves are inside the loop's input
                // window; the second ends at `$off + 12`.
                let lo = unsafe { _mm_loadu_si128(input.as_ptr().add(start).cast()) };
                let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(start + 16).cast()) };
                _mm256_set_m128i(hi, lo)
            } else {
                // SAFETY: the loop condition covers the 32-byte window.
                unsafe { _mm256_loadu_si256(input.as_ptr().add(start).cast()) }
            }
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shifted);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        // `at` is 24 modulo 64 throughout this loop (192-byte stride).
        let b0 = window!(at, false);
        let b1 = window!(at + 24, true);
        let b2 = window!(at + 48, false);
        let b3 = window!(at + 72, false);
        let b4 = window!(at + 96, true);
        let b5 = window!(at + 120, false);
        let b6 = window!(at + 144, true);
        let b7 = window!(at + 168, true);
        let r0 = convert!(b0);
        let r1 = convert!(b1);
        let r2 = convert!(b2);
        let r3 = convert!(b3);
        let r4 = convert!(b4);
        let r5 = convert!(b5);
        let r6 = convert!(b6);
        let r7 = convert!(b7);
        // SAFETY: eight groups reserve eight 32-byte stores.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 128).cast(), r4);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 160).cast(), r5);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 192).cast(), r6);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 224).cast(), r7);
        }
        at += 192;
        out += 256;
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Sequential eight-block unroll: two four-block groups per loop iteration,
/// with only one group live at a time. This halves loop-control overhead
/// without the register pressure of keeping eight values alive.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_unroll8_seq(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! group4 {
        () => {{
            // SAFETY: the loop condition guarantees four 32-byte windows.
            let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 20).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 44).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 68).cast()) };
            macro_rules! convert {
                ($bytes:expr) => {{
                    let x = _mm256_shuffle_epi8($bytes, shifted);
                    // SAFETY: AVX2 is available.
                    let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
                    let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
                    let values = _mm256_or_si256(high, low);
                    let class = _mm256_sub_epi8(
                        _mm256_subs_epu8(values, fifty_one),
                        _mm256_cmpgt_epi8(values, twenty_five),
                    );
                    _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
                }};
            }
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            let r2 = convert!(b2);
            let r3 = convert!(b3);
            // SAFETY: four groups reserve four 32-byte stores.
            unsafe {
                _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
            }
            at += 96;
            out += 128;
        }};
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        group4!();
        group4!();
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Encoder that uses a 32-byte load when the window stays inside one 64-byte
/// cache line and two 16-byte loads when it would cross. Crossing 32-byte
/// loads split on the load ports; avoiding them costs an extra load and insert
/// only for the crossing blocks.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_hybrid(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let limit = input.len().saturating_sub(24);
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! load_window {
        ($offset:expr) => {{
            let start = $offset - 4;
            if start & 63 <= 32 {
                // SAFETY: the loop bound guarantees the 32-byte window.
                unsafe { _mm256_loadu_si256(input.as_ptr().add(start).cast()) }
            } else {
                // SAFETY: both 16-byte halves lie in the same window.
                let lo = unsafe { _mm_loadu_si128(input.as_ptr().add(start).cast()) };
                let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(start + 16).cast()) };
                _mm256_set_m128i(hi, lo)
            }
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shifted);
            // SAFETY: AVX2 is available.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            let values = _mm256_or_si256(high, low);
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8(values, fifty_one),
                _mm256_cmpgt_epi8(values, twenty_five),
            );
            _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    while at <= limit && input.len() - at >= 96 {
        if input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        let b0 = load_window!(at);
        let b1 = load_window!(at + 24);
        let b2 = load_window!(at + 48);
        let b3 = load_window!(at + 72);
        let r0 = convert!(b0);
        let r1 = convert!(b1);
        let r2 = convert!(b2);
        let r3 = convert!(b3);
        // SAFETY: four groups reserve four 32-byte stores.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
        }
        at += 96;
        out += 128;
    }
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Encoder with a precomputed chunk count and split prefetch/no-prefetch
/// loops, so the hot loop has no per-iteration prefetch branch.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_countloop(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let chunks = (input.len() - at) / 96;
    let prefetch_chunks = chunks.saturating_sub(ENCODE_PREFETCH / 96 + 1);

    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! chunk {
        ($prefetch:expr) => {{
            if $prefetch {
                // SAFETY: the split loop guarantees at least ENCODE_PREFETCH
                // bytes remain.
                unsafe {
                    _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0)
                };
            }
            // SAFETY: the loop count guarantees 96 readable bytes.
            let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 20).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 44).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 68).cast()) };
            macro_rules! convert {
                ($bytes:expr) => {{
                    let x = _mm256_shuffle_epi8($bytes, shifted);
                    // SAFETY: AVX2 is available.
                    let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
                    let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
                    let values = _mm256_or_si256(high, low);
                    let class = _mm256_sub_epi8(
                        _mm256_subs_epu8(values, fifty_one),
                        _mm256_cmpgt_epi8(values, twenty_five),
                    );
                    _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
                }};
            }
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            let r2 = convert!(b2);
            let r3 = convert!(b3);
            // SAFETY: four groups reserve four 32-byte stores.
            unsafe {
                _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
                _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
            }
            at += 96;
            out += 128;
        }};
    }
    for _ in 0..prefetch_chunks {
        chunk!(true);
    }
    for _ in prefetch_chunks..chunks {
        chunk!(false);
    }
    // SAFETY: the remainder follows the same output contract.
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

/// Eight-block unroll of the 32-byte-load encoder. The remainder is delegated
/// to [`encode`].
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_unroll8(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    let mut at = 0usize;
    let mut out = 0usize;
    let mul_high = std::hint::black_box(_mm256_set1_epi32(0x04000040));
    let mul_low = std::hint::black_box(_mm256_set1_epi32(0x01000010));
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19, -16),
        Alphabet::UrlSafe => (-17, 32),
    };
    let offsets = _mm256_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0, 65, 71, -4, -4, -4, -4, -4,
        -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let shifted = _mm256_setr_epi8(
        5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14, 1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7,
        10, 9, 11, 10,
    );
    let unshifted = _mm256_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10, 5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11,
        14, 13, 15, 14,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);
    macro_rules! sextets {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shifted);
            // SAFETY: this kernel only runs after AVX2 detection.
            let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
            let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
            _mm256_or_si256(high, low)
        }};
    }
    macro_rules! symbols {
        ($values:expr) => {{
            let class = _mm256_sub_epi8(
                _mm256_subs_epu8($values, fifty_one),
                _mm256_cmpgt_epi8($values, twenty_five),
            );
            _mm256_add_epi8($values, _mm256_shuffle_epi8(offsets, class))
        }};
    }
    if input.len() >= 24 {
        // The first group cannot read four bytes before the input.
        // SAFETY: 24 bytes are available.
        let lo = unsafe { _mm_loadu_si128(input.as_ptr().cast()) };
        // SAFETY: the second load covers offsets 8 through 23.
        let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(8).cast()) };
        let bytes = _mm256_set_m128i(hi, lo);
        let x = _mm256_shuffle_epi8(bytes, unshifted);
        // SAFETY: AVX2 is available.
        let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
        let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
        let values = _mm256_or_si256(high, low);
        let class = _mm256_sub_epi8(
            _mm256_subs_epu8(values, fifty_one),
            _mm256_cmpgt_epi8(values, twenty_five),
        );
        let result = _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class));
        // SAFETY: the caller guarantees the first 32 output bytes.
        unsafe { _mm256_storeu_si256(output.as_mut_ptr().cast(), result) };
        at = 24;
        out = 32;
    }
    while input.len() - at >= 196 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        // SAFETY: 196 bytes remain; each window ends at at+195.
        let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
        let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 20).cast()) };
        let b2 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 44).cast()) };
        let b3 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 68).cast()) };
        let b4 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 92).cast()) };
        let b5 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 116).cast()) };
        let b6 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 140).cast()) };
        let b7 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 164).cast()) };
        let v0 = sextets!(b0);
        let v1 = sextets!(b1);
        let v2 = sextets!(b2);
        let v3 = sextets!(b3);
        let v4 = sextets!(b4);
        let v5 = sextets!(b5);
        let v6 = sextets!(b6);
        let v7 = sextets!(b7);
        let r0 = symbols!(v0);
        let r1 = symbols!(v1);
        let r2 = symbols!(v2);
        let r3 = symbols!(v3);
        let r4 = symbols!(v4);
        let r5 = symbols!(v5);
        let r6 = symbols!(v6);
        let r7 = symbols!(v7);
        // SAFETY: the caller guarantees 32 output bytes per group.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 128).cast(), r4);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 160).cast(), r5);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 192).cast(), r6);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 224).cast(), r7);
        }
        at += 192;
        out += 256;
    }
    // SAFETY: the remaining input and output regions follow the same contract.
    let done = unsafe { encode(&input[at..], &mut output[out..], alphabet) };
    at + done
}

/// Decode an arbitrary multiple-of-four prefix in four-block batches. Error
/// markers are accumulated across four blocks and checked once; valid input
/// dominates and the rare failure is located by re-checking the four vectors.
///
/// # Safety
/// AVX2 must be available. `output` must hold `input.len() / 4 * 3` bytes.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn decode_batch4(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> Result<usize, usize> {
    let (low_table, split, roll): (&[u8; 16], i8, [i8; 16]) = match alphabet {
        Alphabet::Standard => (
            &STANDARD_MEMBERSHIP,
            46,
            [0, 0, 19, 16, 4, -65, -65, -71, -71, 0, 0, 0, 0, 0, 0, 0],
        ),
        Alphabet::UrlSafe => (
            &URL_MEMBERSHIP,
            94,
            [0, 0, 17, 4, -65, -65, -32, -71, -71, 0, 0, 0, 0, 0, 0, 0],
        ),
    };
    // SAFETY: each table contains exactly 16 bytes.
    let low_lut =
        unsafe { _mm256_broadcastsi128_si256(_mm_loadu_si128(low_table.as_ptr().cast())) };
    // SAFETY: the roll table contains exactly 16 bytes.
    let roll_lut = unsafe { _mm256_broadcastsi128_si256(_mm_loadu_si128(roll.as_ptr().cast())) };
    let high_lut = _mm256_setr_epi8(
        1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 64, -128, 0, 0,
        0, 0, 0, 0, 0, 0,
    );
    let pair_mul = _mm256_set1_epi16(0x0140);
    let quad_mul = _mm256_set1_epi32(0x0001_1000);
    let shuffle = _mm256_setr_epi8(
        2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13,
        12, -1, -1, -1, -1,
    );
    let reorder = _mm256_setr_epi32(0, 1, 2, 4, 2, 4, 5, 6);

    macro_rules! translate_at {
        ($offset:expr) => {{
            // SAFETY: the caller guarantees the 32-byte window is in-bounds.
            let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add($offset).cast()) };
            let high = _mm256_and_si256(_mm256_srli_epi16(bytes, 4), _mm256_set1_epi8(15));
            let valid = _mm256_and_si256(
                _mm256_shuffle_epi8(low_lut, bytes),
                _mm256_shuffle_epi8(high_lut, high),
            );
            let bad = _mm256_cmpeq_epi8(valid, _mm256_setzero_si256());
            let index = _mm256_sub_epi8(high, _mm256_cmpgt_epi8(bytes, _mm256_set1_epi8(split)));
            let values = _mm256_add_epi8(bytes, _mm256_shuffle_epi8(roll_lut, index));
            (values, bad)
        }};
    }
    macro_rules! store_at {
        ($values:expr, $offset:expr) => {{
            let pairs = _mm256_maddubs_epi16($values, pair_mul);
            let groups = _mm256_madd_epi16(pairs, quad_mul);
            let packed = _mm256_shuffle_epi8(groups, shuffle);
            let reordered = _mm256_permutevar8x32_epi32(packed, reorder);
            // SAFETY: each block consumes four input bytes per output byte and
            // the caller guarantees the 24-byte output region.
            unsafe {
                _mm_storeu_si128(
                    output.as_mut_ptr().add($offset).cast(),
                    _mm256_castsi256_si128(reordered),
                );
                _mm_storeu_si128(
                    output.as_mut_ptr().add($offset + 8).cast(),
                    _mm256_extracti128_si256::<1>(reordered),
                );
            }
        }};
    }
    macro_rules! store_at_next {
        ($values:expr, $offset:expr) => {{
            let pairs = _mm256_maddubs_epi16($values, pair_mul);
            let groups = _mm256_madd_epi16(pairs, quad_mul);
            let packed = _mm256_shuffle_epi8(groups, shuffle);
            // SAFETY: four extra bytes spill into the next block's output,
            // which is either a later full block or the overlapping block,
            // and is overwritten there. The caller guarantees the output.
            unsafe {
                _mm_storeu_si128(
                    output.as_mut_ptr().add($offset).cast(),
                    _mm256_castsi256_si128(packed),
                );
                _mm_storeu_si128(
                    output.as_mut_ptr().add($offset + 12).cast(),
                    _mm256_extracti128_si256::<1>(packed),
                );
            }
        }};
    }
    macro_rules! locate {
        ($a:expr, $b:expr, $c:expr, $d:expr, $block:expr) => {{
            for (i, &bad) in [&$a, &$b, &$c, &$d].into_iter().enumerate() {
                let mask = _mm256_movemask_epi8(bad) as u32;
                if mask != 0 {
                    return Err(($block + i) * 32 + mask.trailing_zeros() as usize);
                }
            }
            unreachable!()
        }};
    }

    let blocks = input.len() / 32;
    let rem = input.len() % 32;
    let mut block = 0usize;
    while block + 4 <= blocks {
        let (v0, b0) = translate_at!(block * 32);
        let (v1, b1) = translate_at!(block * 32 + 32);
        let (v2, b2) = translate_at!(block * 32 + 64);
        let (v3, b3) = translate_at!(block * 32 + 96);
        let bad = _mm256_or_si256(_mm256_or_si256(b0, b1), _mm256_or_si256(b2, b3));
        if _mm256_testz_si256(bad, bad) == 0 {
            locate!(b0, b1, b2, b3, block);
        }
        if block + 1 < blocks || rem >= 8 {
            store_at_next!(v0, block * 24);
        } else {
            store_at!(v0, block * 24);
        }
        if block + 2 < blocks || rem >= 8 {
            store_at_next!(v1, block * 24 + 24);
        } else {
            store_at!(v1, block * 24 + 24);
        }
        if block + 3 < blocks || rem >= 8 {
            store_at_next!(v2, block * 24 + 48);
        } else {
            store_at!(v2, block * 24 + 48);
        }
        if block + 4 < blocks || rem >= 8 {
            store_at_next!(v3, block * 24 + 72);
        } else {
            store_at!(v3, block * 24 + 72);
        }
        block += 4;
    }
    while block < blocks {
        let (values, bad) = translate_at!(block * 32);
        let mask = _mm256_movemask_epi8(bad) as u32;
        if mask != 0 {
            return Err(block * 32 + mask.trailing_zeros() as usize);
        }
        if block + 1 < blocks || rem >= 8 {
            store_at_next!(values, block * 24);
        } else {
            store_at!(values, block * 24);
        }
        block += 1;
    }
    if rem != 0 {
        let start = input.len() - 32;
        let (values, bad) = translate_at!(start);
        let mask = _mm256_movemask_epi8(bad) as u32;
        if mask != 0 {
            return Err(start + mask.trailing_zeros() as usize);
        }
        store_at!(values, start / 4 * 3);
    }
    Ok(input.len() / 4 * 3)
}

#[cfg(target_os = "linux")]
#[repr(C, align(32))]
struct EncodeParams {
    mul_high: [u8; 32],
    mul_low: [u8; 32],
    offsets: [u8; 32],
    shuffle: [u8; 32],
    mask_high: [u8; 32],
    mask_low: [u8; 32],
    fifty_one: u8,
    twenty_five: u8,
    _pad: [u8; 30],
}

#[cfg(target_os = "linux")]
const fn repeat4(bytes: [u8; 4]) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut i = 0;
    while i < 8 {
        out[i * 4] = bytes[0];
        out[i * 4 + 1] = bytes[1];
        out[i * 4 + 2] = bytes[2];
        out[i * 4 + 3] = bytes[3];
        i += 1;
    }
    out
}

#[cfg(target_os = "linux")]
const fn encode_params(url: bool) -> EncodeParams {
    let (s62, s63) = if url { (-17i8, 32i8) } else { (-19i8, -16i8) };
    let mut offsets = [0u8; 32];
    let mut i = 0;
    while i < 2 {
        let mut j = 0;
        while j < 16 {
            offsets[i * 16 + j] = if i == 0 && j == 0 {
                65
            } else if i == 0 && j == 1 {
                71
            } else if i == 0 && (j == 12 || j == 13) {
                if j == 12 { s62 as u8 } else { s63 as u8 }
            } else if i == 0 && j >= 14 {
                0
            } else if i == 1 && j == 0 {
                65
            } else if i == 1 && j == 1 {
                71
            } else if i == 1 && (j == 12 || j == 13) {
                if j == 12 { s62 as u8 } else { s63 as u8 }
            } else if i == 1 && j >= 14 {
                0
            } else {
                -4i8 as u8
            };
            j += 1;
        }
        i += 1;
    }
    let mut shuffle = [0u8; 32];
    let first = [1u8, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10];
    let second = [5u8, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11, 14, 13, 15, 14];
    let mut k = 0;
    while k < 16 {
        shuffle[k] = first[k];
        shuffle[16 + k] = second[k];
        k += 1;
    }
    EncodeParams {
        mul_high: repeat4([0x40, 0x00, 0x00, 0x04]),
        mul_low: repeat4([0x10, 0x00, 0x00, 0x01]),
        offsets,
        shuffle,
        mask_high: repeat4([0x00, 0xfc, 0xc0, 0x0f]),
        mask_low: repeat4([0xf0, 0x03, 0x3f, 0x00]),
        fifty_one: 51,
        twenty_five: 25,
        _pad: [0u8; 30],
    }
}

#[cfg(target_os = "linux")]
static STANDARD_ENCODE_PARAMS: EncodeParams = encode_params(false);
#[cfg(target_os = "linux")]
static URL_ENCODE_PARAMS: EncodeParams = encode_params(true);

#[cfg(target_os = "linux")]
unsafe extern "C" {
    fn yolow64_encode_load16_groups8(
        src: *const u8,
        dst: *mut u8,
        chunks: usize,
        params: *const u8,
    );
}

#[cfg(target_os = "linux")]
global_asm!(
    r#"
    .globl yolow64_encode_load16_groups8
    .hidden yolow64_encode_load16_groups8
    .type yolow64_encode_load16_groups8,@function
    .p2align 6
yolow64_encode_load16_groups8:
    test %rdx, %rdx
    jz .Lenc_done
    vmovdqa 0(%rcx), %ymm0
    vmovdqa 32(%rcx), %ymm1
    vmovdqa 64(%rcx), %ymm2
    vmovdqa 96(%rcx), %ymm3
    vmovdqa 128(%rcx), %ymm4
    vmovdqa 160(%rcx), %ymm5
    vpbroadcastb 192(%rcx), %ymm6
    vpbroadcastb 193(%rcx), %ymm7
    .p2align 5
.Lenc_loop:
    prefetcht0 8192(%rdi)
    .macro ENC off, oout
    vmovdqu \off(%rdi), %xmm8
    vinserti128 $1, \off+8(%rdi), %ymm8, %ymm8
    vpshufb %ymm3, %ymm8, %ymm8
    vpand %ymm4, %ymm8, %ymm12
    vpmulhuw %ymm0, %ymm12, %ymm12
    vpand %ymm5, %ymm8, %ymm8
    vpmullw %ymm1, %ymm8, %ymm8
    vpor %ymm8, %ymm12, %ymm8
    vpsubusb %ymm6, %ymm8, %ymm12
    vpcmpgtb %ymm7, %ymm8, %ymm13
    vpsubb %ymm13, %ymm12, %ymm12
    vpshufb %ymm12, %ymm2, %ymm12
    vpaddb %ymm8, %ymm12, %ymm8
    vmovdqu %ymm8, \oout(%rsi)
    .endm
    ENC 0, 0
    ENC 24, 32
    ENC 48, 64
    ENC 72, 96
    ENC 96, 128
    ENC 120, 160
    ENC 144, 192
    ENC 168, 224
    add $192, %rdi
    add $256, %rsi
    dec %rdx
    jnz .Lenc_loop
.Lenc_done:
    vzeroupper
    ret
    .size yolow64_encode_load16_groups8, .-yolow64_encode_load16_groups8
"#,
    options(att_syntax)
);

#[cfg(target_os = "linux")]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_asm_load16(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let chunks = (input.len() - 24) / 192;
    if chunks != 0 {
        let params = match alphabet {
            Alphabet::Standard => &STANDARD_ENCODE_PARAMS,
            Alphabet::UrlSafe => &URL_ENCODE_PARAMS,
        };
        // SAFETY: the chunk count guarantees each 8-block iteration reads
        // 192 bytes, and the caller guarantees 256 output bytes per chunk.
        unsafe {
            yolow64_encode_load16_groups8(
                input.as_ptr().add(24),
                output.as_mut_ptr().add(32).cast(),
                chunks,
                params as *const EncodeParams as *const u8,
            );
        }
    }
    let at = 24 + chunks * 192;
    let out = 32 + chunks * 256;
    at + unsafe { encode(&input[at..], &mut output[out..], alphabet) }
}

mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn turbo8_encode_matches_reference() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let input: Vec<u8> = (0..(1 << 20)).map(|i| (i * 73 + 19) as u8).collect();
        let mut output = vec![std::mem::MaybeUninit::<u8>::uninit(); input.len() / 3 * 4 + 4];
        // SAFETY: AVX2 was detected; the output has room for all complete groups.
        let at = unsafe { encode_load16_turbo8(&input, &mut output, Alphabet::Standard) };
        let encoded_len = at / 3 * 4;
        // SAFETY: the encoder initialized the complete-group output.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.as_ptr().cast::<u8>(), encoded_len) };
        let expected = base64::engine::general_purpose::STANDARD.encode(&input[..at]);
        assert_eq!(bytes, expected.as_bytes());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn asm_encode_load16_matches_reference() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let input: Vec<u8> = (0..(1 << 20)).map(|i| (i * 73 + 19) as u8).collect();
        let mut output = vec![std::mem::MaybeUninit::<u8>::uninit(); input.len() / 3 * 4 + 4];
        // SAFETY: AVX2 was detected; the output has room for all complete groups.
        let at = unsafe { encode_asm_load16(&input, &mut output, Alphabet::Standard) };
        let encoded_len = at / 3 * 4;
        // SAFETY: `encode_asm_load16` initialized the complete-group output.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.as_ptr().cast::<u8>(), encoded_len) };
        let expected = base64::engine::general_purpose::STANDARD.encode(&input[..at]);
        assert_eq!(bytes, expected.as_bytes());
    }
}
