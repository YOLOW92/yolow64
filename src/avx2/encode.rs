//! AVX2 encoding kernels.

use crate::Alphabet;
use std::arch::asm;
use std::arch::x86_64::*;
use std::mem::MaybeUninit;

/// Distance used by the encode prefetch experiment. Zero disables it.
pub(crate) const ENCODE_PREFETCH: usize = 8192;

/// LLVM 22 lowered the `_mm256_mulhi_epu16` intrinsic on this target into
/// widening, packing and lane permutation. The instruction exists, so pin it.
///
/// # Safety
/// AVX2 must be available.
#[inline]
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn mulhi_epu16(a: __m256i, b: __m256i) -> __m256i {
    let out: __m256i;
    // SAFETY: the caller guarantees AVX2; this instruction has no memory
    // operands, reads only the two input registers and writes one register.
    // Rust's inline assembly defaults to Intel syntax: the destination comes
    // first. Tie it to the first input so it cannot be mistaken for a source.
    unsafe {
        asm!(
            "vpmulhuw {a}, {b}, {a}",
            a = inout(ymm_reg) a => out,
            b = in(ymm_reg) b,
            options(pure, nomem, nostack, preserves_flags)
        );
    }
    out
}

/// Eight-block sequential loop with the prefetch branch split out. The hot
/// loop runs a fixed chunk count, so it has no per-iteration prefetch compare
/// and no extra jump into the body.
///
/// `NT` selects streaming stores: `0` uses cached stores, `1` uses 32-byte
/// streaming stores, and `2` uses two 16-byte streaming stores per vector.
/// Streaming is only useful when the output is evicted from the last-level
/// cache before it is reused. Callers must pass an output pointer aligned to
/// 32 bytes for `NT = 1` and 16 bytes for `NT = 2` at the first streamed store.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
/// `output.as_mut_ptr().add(32)` must meet the alignment above when `NT != 0`.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_unroll8_split<const NT: u8>(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    if input.len() < 24 {
        return unsafe { encode(input, output, alphabet) };
    }
    let first = unsafe { encode_load16(&input[..24], &mut output[..32], alphabet) };
    debug_assert_eq!(first, 24);
    let mut at = 24usize;
    let mut out = 32usize;
    let chunks = (input.len() - at) / 192;
    #[cfg(test)]
    let prefetch_distance =
        super::lab::ENCODE_PREFETCH_DISTANCE.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let prefetch_distance = ENCODE_PREFETCH;
    #[cfg(test)]
    let prefetch_nta = super::lab::ENCODE_PREFETCH_NTA.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let prefetch_nta = false;
    #[cfg(test)]
    let prefetch_wide = super::lab::ENCODE_PREFETCH_WIDE.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let prefetch_wide = true;
    let prefetch_chunks = if prefetch_distance == 0 {
        0
    } else {
        chunks.saturating_sub(prefetch_distance / 192 + 1)
    };
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
    macro_rules! group4 {
        () => {{
            // SAFETY: the chunk count guarantees four 32-byte windows.
            let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
            let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 20).cast()) };
            let b2 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 44).cast()) };
            let b3 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 68).cast()) };
            let r0 = convert!(b0);
            let r1 = convert!(b1);
            let r2 = convert!(b2);
            let r3 = convert!(b3);
            // SAFETY: four groups reserve four 32-byte stores.
            unsafe {
                match NT {
                    1 => {
                        _mm256_stream_si256(output.as_mut_ptr().add(out).cast(), r0);
                        _mm256_stream_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
                        _mm256_stream_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
                        _mm256_stream_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
                    }
                    2 => {
                        for (offset, value) in
                            [(out, r0), (out + 32, r1), (out + 64, r2), (out + 96, r3)]
                        {
                            let half = output.as_mut_ptr().add(offset).cast::<__m128i>();
                            _mm_stream_si128(half, _mm256_castsi256_si128(value));
                            _mm_stream_si128(half.add(1), _mm256_extracti128_si256(value, 1));
                        }
                    }
                    _ => {
                        _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
                        _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
                        _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
                        _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
                    }
                }
            }
            at += 96;
            out += 128;
        }};
    }
    for _ in 0..prefetch_chunks {
        // One iteration consumes 192 input bytes = three 64-byte lines. The
        // narrow form prefetches one line and lets the hardware prefetcher
        // cover the rest; the wide form covers all three explicitly.
        // SAFETY: the chunk count leaves at least `prefetch_distance` bytes.
        let ptr = unsafe { input.as_ptr().add(at + prefetch_distance) }.cast::<i8>();
        macro_rules! prefetch_line {
            ($offset:expr) => {
                // SAFETY: the chunk count leaves at least `prefetch_distance`
                // bytes, and the wide form adds only 128 bytes.
                unsafe {
                    if prefetch_nta {
                        _mm_prefetch(ptr.add($offset), _MM_HINT_NTA)
                    } else {
                        _mm_prefetch(ptr.add($offset), _MM_HINT_T0)
                    }
                }
            };
        }
        prefetch_line!(0);
        if prefetch_wide {
            prefetch_line!(64);
            prefetch_line!(128);
        }
        group4!();
        group4!();
    }
    for _ in prefetch_chunks..chunks {
        group4!();
        group4!();
    }
    let written = at + unsafe { encode(&input[at..], &mut output[out..], alphabet) };
    if NT != 0 {
        // SAFETY: AVX2 implies SSE2, so `sfence` is available. It is safe on
        // newer toolchains and unsafe on the MSRV, hence the allow.
        #[allow(unused_unsafe)]
        unsafe {
            _mm_sfence()
        };
    }
    written
}

/// Encode one 12-byte chunk into exactly 16 characters using SSSE3/AVX2.
/// The caller supplies a zero-padded 16-byte staging buffer.
///
/// # Safety
/// AVX2 must be available.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn encode_12_bytes(chunk: &[u8; 12], alphabet: Alphabet) -> __m128i {
    let (s62, s63) = match alphabet {
        Alphabet::Standard => (-19i8, -16i8),
        Alphabet::UrlSafe => (-17i8, 32i8),
    };
    let mut stage = [0u8; 16];
    stage[..12].copy_from_slice(chunk);
    // SAFETY: the staging array is 16 bytes.
    let bytes = unsafe { _mm_loadu_si128(stage.as_ptr().cast()) };
    let vh = _mm_setr_epi8(1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10);
    let mask_high = _mm_set1_epi32(0x0fc0fc00);
    let mask_low = _mm_set1_epi32(0x003f_03f0);
    let offsets = _mm_setr_epi8(
        65, 71, -4, -4, -4, -4, -4, -4, -4, -4, -4, -4, s62, s63, 0, 0,
    );
    let x = _mm_shuffle_epi8(bytes, vh);
    let high = _mm_mulhi_epu16(_mm_and_si128(x, mask_high), _mm_set1_epi32(0x04000040));
    let low = _mm_mullo_epi16(_mm_and_si128(x, mask_low), _mm_set1_epi32(0x01000010));
    let values = _mm_or_si128(high, low);
    let class = _mm_sub_epi8(
        _mm_subs_epu8(values, _mm_set1_epi8(51)),
        _mm_cmpgt_epi8(values, _mm_set1_epi8(25)),
    );
    _mm_add_epi8(values, _mm_shuffle_epi8(offsets, class))
}

/// Encode the first 12 bytes into 16 characters.
///
/// # Safety
/// AVX2 must be available, `input.len() >= 12`, and `output` must hold 16 bytes.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_12(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    debug_assert!(input.len() >= 12);
    let chunk: &[u8; 12] = input[..12].try_into().unwrap();
    let result = unsafe { encode_12_bytes(chunk, alphabet) };
    // SAFETY: the caller guarantees 16 output bytes.
    unsafe { _mm_storeu_si128(output.as_mut_ptr().cast(), result) };
    16
}

/// Convert the 24-byte layout to 32 Base64 characters.
///
/// # Safety
/// AVX2 must be available.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn convert_24(bytes: __m256i, alphabet: Alphabet) -> __m256i {
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

    let x = _mm256_shuffle_epi8(bytes, shuffle);
    // SAFETY: AVX2 is available.
    let high = unsafe { mulhi_epu16(_mm256_and_si256(x, mask_high), mul_high) };
    let low = _mm256_mullo_epi16(_mm256_and_si256(x, mask_low), mul_low);
    let values = _mm256_or_si256(high, low);
    let class = _mm256_sub_epi8(
        _mm256_subs_epu8(values, fifty_one),
        _mm256_cmpgt_epi8(values, twenty_five),
    );
    _mm256_add_epi8(values, _mm256_shuffle_epi8(offsets, class))
}

/// Encode exactly 8 bytes into 12 padded characters.
///
/// # Safety
/// AVX2 must be available and `output` must hold 12 bytes. The caller must
/// request padding: the result ends in one `=`.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_8(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    debug_assert_eq!(input.len(), 8);
    debug_assert!(output.len() >= 12);
    let lo = unsafe { _mm_loadl_epi64(input.as_ptr().cast()) };
    let bytes = _mm256_set_m128i(_mm_setzero_si128(), lo);
    let result = unsafe { convert_24(bytes, alphabet) };
    let first = _mm256_castsi256_si128(result);
    let tail = _mm_srli_si128::<8>(first);
    // SAFETY: the caller guarantees 12 output bytes.
    unsafe {
        _mm_storel_epi64(output.as_mut_ptr().cast(), first);
        _mm_storeu_si32(output.as_mut_ptr().add(8).cast(), tail);
    }
    output[11].write(b'=');
    12
}

/// Encode exactly 16 bytes into 24 padded characters.
///
/// # Safety
/// AVX2 must be available and `output` must hold 24 bytes. The caller must
/// request padding: the result ends in `==`.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_16(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    debug_assert_eq!(input.len(), 16);
    debug_assert!(output.len() >= 24);
    let lo = unsafe { _mm_loadu_si128(input.as_ptr().cast()) };
    // Bytes 8..16 followed by eight zeros complete the 24-byte conversion
    // layout without reading past the 16-byte input.
    let hi = unsafe { _mm_loadl_epi64(input.as_ptr().add(8).cast()) };
    let bytes = _mm256_set_m128i(hi, lo);
    let result = unsafe { convert_24(bytes, alphabet) };
    let first = _mm256_castsi256_si128(result);
    let second = _mm256_extracti128_si256::<1>(result);
    // SAFETY: the caller guarantees 24 output bytes.
    unsafe {
        _mm_storeu_si128(output.as_mut_ptr().cast(), first);
        _mm_storel_epi64(output.as_mut_ptr().add(16).cast(), second);
    }
    output[22].write(b'=');
    output[23].write(b'=');
    24
}

/// Encode exactly 32 bytes into 44 padded characters.
///
/// # Safety
/// AVX2 must be available and `output` must hold 44 bytes. The caller must
/// request padding: the result ends in one `=`.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_32(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    debug_assert_eq!(input.len(), 32);
    debug_assert!(output.len() >= 44);
    let lo = unsafe { _mm_loadu_si128(input.as_ptr().cast()) };
    let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(8).cast()) };
    let first = unsafe { convert_24(_mm256_set_m128i(hi, lo), alphabet) };
    // SAFETY: the caller guarantees 44 output bytes.
    unsafe { _mm256_storeu_si256(output.as_mut_ptr().cast(), first) };

    let tail_in = unsafe { _mm_loadl_epi64(input.as_ptr().add(24).cast()) };
    let tail_vec = unsafe { convert_24(_mm256_set_m128i(_mm_setzero_si128(), tail_in), alphabet) };
    let tail_first = _mm256_castsi256_si128(tail_vec);
    let tail_last = _mm_srli_si128::<8>(tail_first);
    // SAFETY: output[32..44] is within the promised 44 bytes.
    unsafe {
        _mm_storel_epi64(output.as_mut_ptr().add(32).cast(), tail_first);
        _mm_storeu_si32(output.as_mut_ptr().add(40).cast(), tail_last);
    }
    output[43].write(b'=');
    44
}

/// Encode with two 16-byte loads per 24-byte group instead of one unaligned
/// 32-byte load starting four bytes before the group. This trades four extra
/// load uops per four groups for fewer cache-line-crossing loads.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate 32 bytes per complete
/// 24-byte group.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode_load16(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
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
    let shuffle = _mm256_setr_epi8(
        1, 0, 2, 1, 4, 3, 5, 4, 7, 6, 8, 7, 10, 9, 11, 10, 5, 4, 6, 5, 8, 7, 9, 8, 11, 10, 12, 11,
        14, 13, 15, 14,
    );
    let mask_high = _mm256_set1_epi32(0x0fc0fc00);
    let mask_low = _mm256_set1_epi32(0x003f_03f0);
    let fifty_one = _mm256_set1_epi8(51);
    let twenty_five = _mm256_set1_epi8(25);

    macro_rules! load_pair {
        ($at:expr) => {{
            // SAFETY: the caller guarantees a complete 24-byte group.
            let lo = unsafe { _mm_loadu_si128(input.as_ptr().add($at).cast()) };
            let hi = unsafe { _mm_loadu_si128(input.as_ptr().add($at + 8).cast()) };
            _mm256_set_m128i(hi, lo)
        }};
    }
    macro_rules! convert {
        ($bytes:expr) => {{
            let x = _mm256_shuffle_epi8($bytes, shuffle);
            // SAFETY: this kernel only runs after AVX2 detection.
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
        let b0 = load_pair!(at);
        let b1 = load_pair!(at + 24);
        let b2 = load_pair!(at + 48);
        let b3 = load_pair!(at + 72);
        let r0 = convert!(b0);
        let r1 = convert!(b1);
        let r2 = convert!(b2);
        let r3 = convert!(b3);
        // SAFETY: four complete groups imply room for four 32-byte stores.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
        }
        at += 96;
        out += 128;
    }
    while input.len() - at >= 24 {
        let bytes = load_pair!(at);
        let result = convert!(bytes);
        // SAFETY: one complete group implies room for one 32-byte store.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), result);
        }
        at += 24;
        out += 32;
    }
    let groups = (input.len() - at) / 3;
    if groups > 0 && at + groups * 3 >= 24 {
        let end = at + groups * 3;
        let start = end - 24;
        let bytes = load_pair!(start);
        let result = convert!(bytes);
        let out_end = end / 3 * 4;
        // SAFETY: the overlapping final store ends within the encoded length.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out_end - 32).cast(), result);
        }
        at = end;
    }
    at
}

/// Encode complete 24-byte groups; the caller handles the remainder.
///
/// # Safety
/// The processor must support AVX2, and output must accommodate 32 bytes for
/// each complete 24-byte group processed.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn encode(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
) -> usize {
    let mut at = 0;
    let mut out = 0;
    if input.len() < 24 {
        return 0;
    }
    let limit = input.len() - 24;
    // LLVM otherwise expands constant multiplications into a longer sequence
    // of widening, variable shifts and packing on the tested toolchain.
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
        ($bytes:expr, $shuffle:expr) => {{
            // Rule 1: multiply, shuffle and interval offsets implement the
            // independent 8-bit -> 6-bit repacking in each SIMD lane.
            let x = _mm256_shuffle_epi8($bytes, $shuffle);
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
    macro_rules! convert {
        ($bytes:expr, $shuffle:expr) => {{
            let values = sextets!($bytes, $shuffle);
            symbols!(values)
        }};
    }
    // The first block cannot read four bytes before the input, so it uses two
    // bounded 16-byte loads and an unshifted shuffle. Handling it once also
    // removes the `at == 0` test from the hot loop.
    if input.len() >= 28 {
        // SAFETY: at least 24 bytes remain, so both 16-byte loads fit.
        let lo = unsafe { _mm_loadu_si128(input.as_ptr().cast()) };
        // SAFETY: the second load covers offsets 8 through 23.
        let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(8).cast()) };
        let bytes = _mm256_set_m128i(hi, lo);
        let result = convert!(bytes, unshifted);
        // SAFETY: the caller guarantees room for one complete group.
        unsafe { _mm256_storeu_si256(output.as_mut_ptr().cast(), result) };
        at = 24;
        out = 32;
    }
    // Rule 2: one prefetch per 96 input bytes keeps the source ahead of the
    // hardware streamer once it no longer fits in L1.
    while input.len() - at >= 100 {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        // SAFETY: the four 32-byte windows start at at-4 and 24-byte
        // increments, ending at at+99, which is inside the input.
        let b0 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
        // SAFETY: the loop condition covers this 32-byte window.
        let b1 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 20).cast()) };
        // SAFETY: the loop condition covers this 32-byte window.
        let b2 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 44).cast()) };
        // SAFETY: the loop condition covers this 32-byte window.
        let b3 = unsafe { _mm256_loadu_si256(input.as_ptr().add(at + 68).cast()) };
        // Rule 1: compute the multiplies for all four blocks first, then
        // translate them. Keeping the phases adjacent gives the scheduler
        // four independent chains instead of one long serial chain.
        let v0 = sextets!(b0, shifted);
        let v1 = sextets!(b1, shifted);
        let v2 = sextets!(b2, shifted);
        let v3 = sextets!(b3, shifted);
        let r0 = symbols!(v0);
        let r1 = symbols!(v1);
        let r2 = symbols!(v2);
        let r3 = symbols!(v3);
        // SAFETY: four complete 24-byte input blocks imply room for four
        // 32-byte stores by the caller's output-capacity contract.
        unsafe {
            _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), r0);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 32).cast(), r1);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 64).cast(), r2);
            _mm256_storeu_si256(output.as_mut_ptr().add(out + 96).cast(), r3);
        }
        at += 96;
        out += 128;
    }
    while at <= limit {
        if ENCODE_PREFETCH != 0 && input.len() - at > ENCODE_PREFETCH {
            // SAFETY: the prefetch address is inside the input allocation.
            unsafe { _mm_prefetch(input.as_ptr().add(at + ENCODE_PREFETCH).cast(), _MM_HINT_T0) };
        }
        let (bytes, shuffle) = if input.len() - at < 28 {
            // SAFETY: at least 24 bytes remain, so both 16-byte loads fit;
            // the second starts at offset 8 within the same 24-byte group.
            let lo = unsafe { _mm_loadu_si128(input.as_ptr().add(at).cast()) };
            // SAFETY: the second load covers offsets 8 through 23.
            let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(at + 8).cast()) };
            (_mm256_set_m128i(hi, lo), unshifted)
        } else {
            // SAFETY: at >= 24 and at least 28 bytes remain; this 32-byte
            // window starts four bytes before at and ends inside the slice.
            let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add(at - 4).cast()) };
            (bytes, shifted)
        };
        // Rule 1: 8-bit to 6-bit repacking, in parallel per 24-byte group.
        // Third-party attribution is recorded in THIRD-PARTY.md.
        let result = convert!(bytes, shuffle);
        // SAFETY: each iteration consumes 24 bytes and writes 32; the caller
        // guarantees enough output for all complete groups.
        unsafe { _mm256_storeu_si256(output.as_mut_ptr().add(out).cast(), result) };
        at += 24;
        out += 32;
    }
    // Encode the final complete block once more, shifted so its 32-byte store
    // ends exactly at the output for all complete groups. The overlap rewrites
    // identical bytes and replaces up to seven scalar groups with one SIMD
    // block. Both 16-byte loads stay inside the last 24 bytes.
    let groups = (input.len() - at) / 3;
    if groups > 0 && at + groups * 3 >= 24 {
        let end = at + groups * 3;
        let start = end - 24;
        // SAFETY: start >= 0 and the second load covers start+8..start+23,
        // which is inside the input because end <= input.len().
        let lo = unsafe { _mm_loadu_si128(input.as_ptr().add(start).cast()) };
        // SAFETY: the two loads together cover the complete 24-byte block.
        let hi = unsafe { _mm_loadu_si128(input.as_ptr().add(start + 8).cast()) };
        let bytes = _mm256_set_m128i(hi, lo);
        let result = convert!(bytes, unshifted);
        let out_end = end / 3 * 4;
        // SAFETY: out_end - 32 >= 0 because end >= 24, and out_end is at most
        // the encoded length of the complete groups, which the caller sized.
        unsafe { _mm256_storeu_si256(output.as_mut_ptr().add(out_end - 32).cast(), result) };
        at = end;
    }
    at
}
