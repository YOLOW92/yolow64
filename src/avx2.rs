//! AVX2 bulk decoder. All loads and stores are bounded; the caller handles tails.

use crate::Alphabet;
use std::arch::asm;
#[cfg(target_os = "linux")]
use std::arch::global_asm;
use std::arch::x86_64::*;
use std::mem::MaybeUninit;

/// Distance used by the encode prefetch experiment. Zero disables it.
const ENCODE_PREFETCH: usize = 8192;

const fn compaction_table() -> [[u8; 8]; 256] {
    let mut table = [[0u8; 8]; 256];
    let mut mask = 0usize;
    while mask < 256 {
        let mut out = 0usize;
        let mut i = 0usize;
        while i < 8 {
            if mask & (1 << i) == 0 {
                table[mask][out] = i as u8;
                out += 1;
            }
            i += 1;
        }
        while out < 8 {
            table[mask][out] = 0x80;
            out += 1;
        }
        mask += 1;
    }
    table
}

const fn compaction_counts() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut mask = 0usize;
    while mask < 256 {
        table[mask] = 8 - (mask as u8).count_ones() as u8;
        mask += 1;
    }
    table
}

static COMPACTION: [[u8; 8]; 256] = compaction_table();
static COMPACTION_COUNT: [u8; 256] = compaction_counts();

/// Returns the first `=` byte, and (when `whitespace` is true) the first
/// ASCII space, tab, CR or LF. Used by the streaming decoder to decide whether
/// a chunk can take the bulk path without a scalar pre-scan.
///
/// # Safety
/// AVX2 must be available.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn first_special(input: &[u8], whitespace: bool) -> Option<usize> {
    let eq = _mm256_set1_epi8(b'=' as i8);
    let space = _mm256_set1_epi8(b' ' as i8);
    let tab = _mm256_set1_epi8(b'\t' as i8);
    let cr = _mm256_set1_epi8(b'\r' as i8);
    let lf = _mm256_set1_epi8(b'\n' as i8);
    let mut at = 0usize;
    while input.len() - at >= 32 {
        // SAFETY: the loop condition guarantees 32 readable bytes.
        let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add(at).cast()) };
        let mut matches = _mm256_cmpeq_epi8(bytes, eq);
        if whitespace {
            matches = _mm256_or_si256(
                matches,
                _mm256_or_si256(
                    _mm256_cmpeq_epi8(bytes, space),
                    _mm256_or_si256(
                        _mm256_cmpeq_epi8(bytes, tab),
                        _mm256_or_si256(_mm256_cmpeq_epi8(bytes, cr), _mm256_cmpeq_epi8(bytes, lf)),
                    ),
                ),
            );
        }
        let mask = _mm256_movemask_epi8(matches) as u32;
        if mask != 0 {
            return Some(at + mask.trailing_zeros() as usize);
        }
        at += 32;
    }
    for (i, &b) in input[at..].iter().enumerate() {
        if b == b'=' || (whitespace && matches!(b, b' ' | b'\t' | b'\r' | b'\n')) {
            return Some(at + i);
        }
    }
    None
}

/// Copies a prefix into `output`, omitting space, tab, CR and LF.
///
/// # Safety
/// AVX2 must be available. The returned output prefix is initialized.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn filter_whitespace(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
) -> (usize, usize) {
    // Consuming at most output.len() bytes guarantees room even if no bytes
    // are removed; input and output are disjoint safe references.
    let count = input.len().min(output.len());
    let (mut read, mut written) = (0, 0);
    let space = _mm256_set1_epi8(b' ' as i8);
    let tab = _mm256_set1_epi8(b'\t' as i8);
    let cr = _mm256_set1_epi8(b'\r' as i8);
    let lf = _mm256_set1_epi8(b'\n' as i8);
    while count - read >= 32 {
        // SAFETY: the loop ensures 32 readable input bytes remain.
        let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add(read).cast()) };
        let matches = _mm256_or_si256(
            _mm256_or_si256(
                _mm256_cmpeq_epi8(bytes, space),
                _mm256_cmpeq_epi8(bytes, tab),
            ),
            _mm256_or_si256(_mm256_cmpeq_epi8(bytes, cr), _mm256_cmpeq_epi8(bytes, lf)),
        );
        let mut mask = _mm256_movemask_epi8(matches) as u32;
        if mask == 0 {
            // SAFETY: at most `read` bytes were written so far, and the
            // 32-byte destination lies within the `count`-byte output prefix.
            unsafe { _mm256_storeu_si256(output.as_mut_ptr().add(written).cast(), bytes) };
            written += 32;
        } else if output.len() - written >= 32 {
            // Rule 1: compact eight bytes at a time with a 2 KiB shuffle
            // table. The table moves non-whitespace bytes to the low lanes;
            // the count table supplies the advance. Stores are eight bytes,
            // which is always in bounds because a full 32-byte block of
            // output space remains.
            let mut lane = 0usize;
            while lane < 4 {
                let lane_mask = (mask >> (lane * 8)) & 0xff;
                // SAFETY: this lane is inside the current 32-byte input block,
                // and the output has at least 32 bytes of space.
                let chunk = unsafe { _mm_loadl_epi64(input.as_ptr().add(read + lane * 8).cast()) };
                if lane_mask == 0 {
                    // SAFETY: the destination is inside the block's 32-byte
                    // output reservation.
                    unsafe {
                        _mm_storel_epi64(output.as_mut_ptr().add(written).cast(), chunk);
                    }
                    written += 8;
                } else {
                    let shuffle = COMPACTION[lane_mask as usize];
                    // SAFETY: `shuffle` is eight bytes; loadl zero-extends it.
                    let shuffle = unsafe { _mm_loadl_epi64(shuffle.as_ptr().cast()) };
                    let packed = _mm_shuffle_epi8(chunk, shuffle);
                    // SAFETY: the destination is inside the block's output
                    // reservation; the store always writes eight bytes.
                    unsafe {
                        _mm_storel_epi64(output.as_mut_ptr().add(written).cast(), packed);
                    }
                    written += COMPACTION_COUNT[lane_mask as usize] as usize;
                }
                lane += 1;
            }
        } else {
            // Near the end of the caller's output there may not be room for
            // an eight-byte store. Fall back to exact copies.
            let mut start = 0;
            while mask != 0 {
                let end = mask.trailing_zeros() as usize;
                let len = end - start;
                // SAFETY: both ranges fit inside the current 32-byte input
                // block and the output prefix; the slices cannot overlap.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        input.as_ptr().add(read + start),
                        output.as_mut_ptr().add(written).cast::<u8>(),
                        len,
                    );
                }
                written += len;
                start = end + 1;
                mask &= mask - 1;
            }
            let len = 32 - start;
            // SAFETY: the remaining suffix is within the current block and
            // the output prefix; the slices cannot overlap.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    input.as_ptr().add(read + start),
                    output.as_mut_ptr().add(written).cast::<u8>(),
                    len,
                );
            }
            written += len;
        }
        read += 32;
    }
    for &byte in &input[read..count] {
        if !matches!(byte, b' ' | b'\t' | b'\r' | b'\n') {
            output[written].write(byte);
            written += 1;
        }
    }
    (count, written)
}

/// LLVM 22 lowered the `_mm256_mulhi_epu16` intrinsic on this target into
/// widening, packing and lane permutation. The instruction exists, so pin it.
///
/// # Safety
/// AVX2 must be available.
#[inline]
#[target_feature(enable = "avx2")]
unsafe fn mulhi_epu16(a: __m256i, b: __m256i) -> __m256i {
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

const fn membership(url: bool) -> [u8; 16] {
    let mut table = [0; 16];
    let mut byte = 0u8;
    while byte < 128 {
        if (byte >= b'A' && byte <= b'Z')
            || (byte >= b'a' && byte <= b'z')
            || (byte >= b'0' && byte <= b'9')
            || (!url && (byte == b'+' || byte == b'/'))
            || (url && (byte == b'-' || byte == b'_'))
        {
            table[(byte & 15) as usize] |= 1 << (byte >> 4);
        }
        byte += 1;
    }
    table
}
const STANDARD_MEMBERSHIP: [u8; 16] = membership(false);
const URL_MEMBERSHIP: [u8; 16] = membership(true);

#[cfg(target_os = "linux")]
const fn repeat16(bytes: [u8; 16]) -> [u8; 32] {
    let mut out = [0; 32];
    let mut i = 0;
    while i < 16 {
        out[i] = bytes[i];
        out[i + 16] = bytes[i];
        i += 1;
    }
    out
}

/// Constants for the hash decode loop. Short names; offsets are fixed.
#[repr(C, align(16))]
#[cfg(target_os = "linux")]
struct HashParams {
    d_asso: [u8; 16],
    d_vals: [u8; 16],
    c_asso: [u8; 16],
    c_vals: [u8; 16],
    pair: [u8; 32],
    quad: [u8; 32],
    pack: [u8; 32],
}

#[cfg(target_os = "linux")]
const fn hash_params(
    d_asso: [u8; 16],
    d_vals: [u8; 16],
    c_asso: [u8; 16],
    c_vals: [u8; 16],
) -> HashParams {
    let mut pair = [0u8; 32];
    let mut quad = [0u8; 32];
    let mut i = 0;
    while i < 16 {
        pair[i * 2] = 0x40;
        pair[i * 2 + 1] = 0x01;
        i += 1;
    }
    let mut i = 0;
    while i < 8 {
        quad[i * 4] = 0x00;
        quad[i * 4 + 1] = 0x10;
        quad[i * 4 + 2] = 0x01;
        quad[i * 4 + 3] = 0x00;
        i += 1;
    }
    HashParams {
        d_asso,
        d_vals,
        c_asso,
        c_vals,
        pair,
        quad,
        pack: repeat16([2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, 128, 128, 128, 128]),
    }
}

#[cfg(target_os = "linux")]
static STD_HASH: HashParams = hash_params(
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 12],
    [0, 0, 0, 19, 4, 191, 191, 185, 185, 16, 0, 191, 0, 185, 0, 0],
    [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 6, 10, 10, 10, 6],
    [
        128, 128, 128, 207, 200, 184, 213, 166, 181, 134, 180, 128, 148, 128, 128, 128,
    ],
);

#[cfg(target_os = "linux")]
static URL_HASH: HashParams = hash_params(
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 8],
    [
        0, 0, 0, 17, 4, 191, 191, 185, 185, 191, 224, 185, 0, 0, 0, 0,
    ],
    [7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 4, 4, 8, 4, 4],
    [
        128, 128, 128, 207, 200, 184, 182, 211, 161, 179, 128, 147, 128, 128, 128, 128,
    ],
);

#[cfg(target_os = "linux")]
unsafe extern "C" {
    /// Decode `groups` groups of four blocks. Returns `usize::MAX` on success,
    /// `usize::MAX - 1` on an invalid byte.
    fn decode_hash4(src: *const u8, dst: *mut u8, groups: usize, params: *const u8) -> usize;
}

#[cfg(target_os = "linux")]
global_asm!(
    r#"
    .globl decode_hash4
    .hidden decode_hash4
    .type decode_hash4,@function
    .p2align 5
decode_hash4:
    mov %rdi, %r9
    test %rdx, %rdx
    jz .Lhdone
    vbroadcasti128 0(%rcx), %ymm8
    vbroadcasti128 16(%rcx), %ymm9
    vbroadcasti128 32(%rcx), %ymm10
    vbroadcasti128 48(%rcx), %ymm11
    vmovdqu 64(%rcx), %ymm12
    vmovdqu 96(%rcx), %ymm13
    vmovdqu 128(%rcx), %ymm14
    vpxor %ymm15, %ymm15, %ymm15
    .p2align 5
.Lhloop:
    prefetcht0 1024(%rdi)
    prefetcht0 1088(%rdi)
    vmovdqu 0(%rdi), %ymm0
    vmovdqu 32(%rdi), %ymm1
    vpsrld $3, %ymm0, %ymm4
    vpsrld $3, %ymm1, %ymm5
    vpshufb %ymm0, %ymm10, %ymm6
    vpshufb %ymm1, %ymm10, %ymm7
    vpavgb %ymm4, %ymm6, %ymm6
    vpavgb %ymm5, %ymm7, %ymm7
    vpshufb %ymm6, %ymm11, %ymm6
    vpshufb %ymm7, %ymm11, %ymm7
    vpaddsb %ymm0, %ymm6, %ymm6
    vpaddsb %ymm1, %ymm7, %ymm7
    vpor %ymm6, %ymm15, %ymm15
    vpor %ymm7, %ymm15, %ymm15
    vpshufb %ymm0, %ymm8, %ymm6
    vpshufb %ymm1, %ymm8, %ymm7
    vpavgb %ymm4, %ymm6, %ymm6
    vpavgb %ymm5, %ymm7, %ymm7
    vpshufb %ymm6, %ymm9, %ymm6
    vpshufb %ymm7, %ymm9, %ymm7
    vpaddb %ymm0, %ymm6, %ymm0
    vpaddb %ymm1, %ymm7, %ymm1
    vpmaddubsw %ymm12, %ymm0, %ymm0
    vpmaddubsw %ymm12, %ymm1, %ymm1
    vpmaddwd %ymm13, %ymm0, %ymm0
    vpmaddwd %ymm13, %ymm1, %ymm1
    vpshufb %ymm14, %ymm0, %ymm0
    vpshufb %ymm14, %ymm1, %ymm1
    vmovdqu %xmm0, 0(%rsi)
    vextracti128 $1, %ymm0, 12(%rsi)
    vmovdqu %xmm1, 24(%rsi)
    vextracti128 $1, %ymm1, 36(%rsi)

    vmovdqu 64(%rdi), %ymm2
    vmovdqu 96(%rdi), %ymm3
    vpsrld $3, %ymm2, %ymm4
    vpsrld $3, %ymm3, %ymm5
    vpshufb %ymm2, %ymm10, %ymm6
    vpshufb %ymm3, %ymm10, %ymm7
    vpavgb %ymm4, %ymm6, %ymm6
    vpavgb %ymm5, %ymm7, %ymm7
    vpshufb %ymm6, %ymm11, %ymm6
    vpshufb %ymm7, %ymm11, %ymm7
    vpaddsb %ymm2, %ymm6, %ymm6
    vpaddsb %ymm3, %ymm7, %ymm7
    vpor %ymm6, %ymm15, %ymm15
    vpor %ymm7, %ymm15, %ymm15
    vpshufb %ymm2, %ymm8, %ymm6
    vpshufb %ymm3, %ymm8, %ymm7
    vpavgb %ymm4, %ymm6, %ymm6
    vpavgb %ymm5, %ymm7, %ymm7
    vpshufb %ymm6, %ymm9, %ymm6
    vpshufb %ymm7, %ymm9, %ymm7
    vpaddb %ymm2, %ymm6, %ymm2
    vpaddb %ymm3, %ymm7, %ymm3
    vpmaddubsw %ymm12, %ymm2, %ymm2
    vpmaddubsw %ymm12, %ymm3, %ymm3
    vpmaddwd %ymm13, %ymm2, %ymm2
    vpmaddwd %ymm13, %ymm3, %ymm3
    vpshufb %ymm14, %ymm2, %ymm2
    vpshufb %ymm14, %ymm3, %ymm3
    vmovdqu %xmm2, 48(%rsi)
    vextracti128 $1, %ymm2, 60(%rsi)
    vmovdqu %xmm3, 72(%rsi)
    vextracti128 $1, %ymm3, 84(%rsi)
    add $128, %rdi
    add $96, %rsi
    dec %rdx
    jnz .Lhloop
    vpmovmskb %ymm15, %eax
    test %eax, %eax
    jnz .Lherr
.Lhdone:
    mov $-1, %rax
    vzeroupper
    ret
.Lherr:
    mov $-2, %rax
    vzeroupper
    ret
    .size decode_hash4, .-decode_hash4
"#,
    options(att_syntax)
);

/// Eight-block loop using the classic 16-byte load layout: low half at the
/// group start and high half at `start + 12`, with the same shuffle in both
/// 128-bit lanes. The high load reads four bytes past the 24-byte group; the
/// loop leaves at least a full block of slack and the remainder is handled by
/// the safe encoder.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[cfg(test)]
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
#[cfg(test)]
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
    let chunks = (input.len() - at) / 192;
    #[cfg(test)]
    let prefetch_distance =
        crate::ENCODE_PREFETCH_DISTANCE.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let prefetch_distance = ENCODE_PREFETCH;
    #[cfg(test)]
    let prefetch_nta = crate::ENCODE_PREFETCH_NTA.load(std::sync::atomic::Ordering::Relaxed);
    #[cfg(not(test))]
    let prefetch_nta = false;
    #[cfg(test)]
    let prefetch_wide = crate::ENCODE_PREFETCH_WIDE.load(std::sync::atomic::Ordering::Relaxed);
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

/// Eight-block loop with four two-block phases. Keeping only two blocks live
/// makes it easier for the scheduler to overlap loads, multiplies and stores.
///
/// # Safety
/// AVX2 must be available and `output` must accommodate the complete groups.
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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
#[cfg(test)]
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

/// Encode one 12-byte chunk into exactly 16 characters using SSSE3/AVX2.
/// The caller supplies a zero-padded 16-byte staging buffer.
///
/// # Safety
/// AVX2 must be available.
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
    output: &mut [std::mem::MaybeUninit<u8>],
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

/// Decode exactly 16 alphabet bytes into 12 bytes, validating them.
///
/// # Safety
/// AVX2 must be available. `output` must hold 12 bytes.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn decode16(
    input: &[u8; 16],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> Result<(), usize> {
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
    // SAFETY: each table is exactly 16 bytes.
    let low_lut = unsafe { _mm_loadu_si128(low_table.as_ptr().cast()) };
    // SAFETY: the roll table is exactly 16 bytes.
    let roll_lut = unsafe { _mm_loadu_si128(roll.as_ptr().cast()) };
    let high_lut = _mm_setr_epi8(1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0);

    // SAFETY: the caller provides a 16-byte input.
    let bytes = unsafe { _mm_loadu_si128(input.as_ptr().cast()) };
    let high = _mm_and_si128(_mm_srli_epi16::<4>(bytes), _mm_set1_epi8(15));
    let valid = _mm_and_si128(
        _mm_shuffle_epi8(low_lut, bytes),
        _mm_shuffle_epi8(high_lut, high),
    );
    let bad = _mm_cmpeq_epi8(valid, _mm_setzero_si128());
    let mask = _mm_movemask_epi8(bad) as u32;
    if mask != 0 {
        return Err(mask.trailing_zeros() as usize);
    }
    let index = _mm_sub_epi8(high, _mm_cmpgt_epi8(bytes, _mm_set1_epi8(split)));
    let values = _mm_add_epi8(bytes, _mm_shuffle_epi8(roll_lut, index));
    let pairs = _mm_maddubs_epi16(values, _mm_set1_epi32(0x0140_0140));
    let groups = _mm_madd_epi16(pairs, _mm_set1_epi32(0x0001_1000));
    let shuffle = _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1);
    let packed = _mm_shuffle_epi8(groups, shuffle);
    // SAFETY: the caller guarantees 12 output bytes.
    unsafe {
        _mm_storel_epi64(output.as_mut_ptr().cast(), packed);
        _mm_storeu_si32(
            output.as_mut_ptr().add(8).cast(),
            _mm_srli_si128::<8>(packed),
        );
    }
    Ok(())
}

/// Decode exactly 24 padded base64 characters to 16 bytes in one call.
///
/// The first 16 characters use the 128-bit table path; the last six use the
/// BMI2 `pext` bit-compaction path. The caller checks the trailing `==` and
/// passes the 256-byte scalar decode table for the selected alphabet.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode24(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    table: *const u8,
) -> Result<usize, crate::Error> {
    if output.len() < 16 {
        return Err(crate::Error::OutputTooSmall);
    }
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
    // First 16 characters: the same 128-bit translation/pack as decode16.
    // SAFETY: each table is exactly 16 bytes and the caller guarantees 24
    // input bytes.
    let low_lut = _mm_loadu_si128(low_table.as_ptr().cast());
    let roll_lut = _mm_loadu_si128(roll.as_ptr().cast());
    let high_lut = _mm_setr_epi8(1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0);
    let bytes = _mm_loadu_si128(input.as_ptr().cast());
    let high = _mm_and_si128(_mm_srli_epi16::<4>(bytes), _mm_set1_epi8(15));
    let valid = _mm_and_si128(
        _mm_shuffle_epi8(low_lut, bytes),
        _mm_shuffle_epi8(high_lut, high),
    );
    let bad = _mm_cmpeq_epi8(valid, _mm_setzero_si128());
    let mask = _mm_movemask_epi8(bad) as u32;
    if mask != 0 {
        let offset = mask.trailing_zeros() as usize;
        return Err(if input[offset] == b'=' {
            crate::Error::InvalidPadding
        } else {
            crate::Error::InvalidByte {
                byte: input[offset],
                offset,
            }
        });
    }
    let index = _mm_sub_epi8(high, _mm_cmpgt_epi8(bytes, _mm_set1_epi8(split)));
    let values = _mm_add_epi8(bytes, _mm_shuffle_epi8(roll_lut, index));
    let pairs = _mm_maddubs_epi16(values, _mm_set1_epi32(0x0140_0140));
    let groups = _mm_madd_epi16(pairs, _mm_set1_epi32(0x0001_1000));
    let shuffle = _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1);
    let packed = _mm_shuffle_epi8(groups, shuffle);
    _mm_storel_epi64(output.as_mut_ptr().cast(), packed);
    _mm_storeu_si32(
        output.as_mut_ptr().add(8).cast(),
        _mm_srli_si128::<8>(packed),
    );

    // Last six characters: load the tail, map through the scalar table, and
    // compact the six-bit fields with pext. The two trailing '=' bytes are
    // checked by the caller and are not part of the word.
    let x = (input.as_ptr().add(16) as *const u64).read_unaligned();
    let y = ((*table.add((x & 255) as usize) as u64) << 40)
        | ((*table.add(((x >> 8) & 255) as usize) as u64) << 32)
        | ((*table.add(((x >> 16) & 255) as usize) as u64) << 24)
        | ((*table.add(((x >> 24) & 255) as usize) as u64) << 16)
        | ((*table.add(((x >> 32) & 255) as usize) as u64) << 8)
        | (*table.add(((x >> 40) & 255) as usize) as u64);
    if y & 0x8080_8080_8080 != 0 {
        for (i, &byte) in input[16..22].iter().enumerate() {
            let offset = i + 16;
            if *table.add(byte as usize) == 0xff {
                return Err(if byte == b'=' {
                    crate::Error::InvalidPadding
                } else {
                    crate::Error::InvalidByte { byte, offset }
                });
            }
        }
        return Err(crate::Error::InvalidPadding);
    }
    let z = std::arch::x86_64::_pext_u64(y, 0x3f3f_3f3f_3f3f);
    if z & 0x0f != 0 {
        return Err(crate::Error::InvalidPadding);
    }
    // SAFETY: the output has 16 bytes and the four new bytes start at 12.
    let p = output.as_mut_ptr().add(12).cast::<u8>();
    p.write((z >> 28) as u8);
    p.add(1).write((z >> 20) as u8);
    p.add(2).write((z >> 12) as u8);
    p.add(3).write((z >> 4) as u8);
    Ok(16)
}

/// Decode exactly 172 padded base64 characters to 128 bytes in one call.
///
/// The first 160 characters are five 32-byte blocks. The last twelve use a
/// `pext8` plus `pext3` tail; the caller checks the final `=` and passes the
/// 256-byte scalar decode table.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode172(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    table: *const u8,
) -> Result<usize, crate::Error> {
    if output.len() < 128 {
        return Err(crate::Error::OutputTooSmall);
    }
    // First four blocks: the hand-scheduled hash loop. It leaves one block
    // for the tail so its four-byte store spill lands in valid output; the
    // fifth block is decoded by the single-block path below.
    #[cfg(target_os = "linux")]
    {
        let params = match alphabet {
            Alphabet::Standard => &STD_HASH as *const HashParams as *const u8,
            Alphabet::UrlSafe => &URL_HASH as *const HashParams as *const u8,
        };
        // SAFETY: AVX2 is guaranteed; four blocks are in bounds and the spill
        // lands in the fifth output block.
        let status = decode_hash4(input.as_ptr(), output.as_mut_ptr().cast(), 1, params);
        if status != usize::MAX {
            let offset = first_invalid(input, alphabet);
            let byte = input[offset];
            return Err(if byte == b'=' {
                crate::Error::InvalidPadding
            } else {
                crate::Error::InvalidByte { byte, offset }
            });
        }
        if let Err(offset) = decode(&input[128..160], &mut output[96..120], alphabet) {
            let offset = offset + 128;
            return Err(if input[offset] == b'=' {
                crate::Error::InvalidPadding
            } else {
                crate::Error::InvalidByte {
                    byte: input[offset],
                    offset,
                }
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        if let Err(offset) = decode(&input[..160], &mut output[..120], alphabet) {
            return Err(if input[offset] == b'=' {
                crate::Error::InvalidPadding
            } else {
                crate::Error::InvalidByte {
                    byte: input[offset],
                    offset,
                }
            });
        }
    }

    // Eight-character tail (six output bytes).
    let x8 = (input.as_ptr().add(160) as *const u64).read_unaligned();
    let y8 = ((*table.add((x8 & 255) as usize) as u64) << 56)
        | ((*table.add(((x8 >> 8) & 255) as usize) as u64) << 48)
        | ((*table.add(((x8 >> 16) & 255) as usize) as u64) << 40)
        | ((*table.add(((x8 >> 24) & 255) as usize) as u64) << 32)
        | ((*table.add(((x8 >> 32) & 255) as usize) as u64) << 24)
        | ((*table.add(((x8 >> 40) & 255) as usize) as u64) << 16)
        | ((*table.add(((x8 >> 48) & 255) as usize) as u64) << 8)
        | (*table.add(((x8 >> 56) & 255) as usize) as u64);
    if y8 & 0x8080_8080_8080_8080 != 0 {
        for (i, &byte) in input[160..168].iter().enumerate() {
            let offset = i + 160;
            if *table.add(byte as usize) == 0xff {
                return Err(if byte == b'=' {
                    crate::Error::InvalidPadding
                } else {
                    crate::Error::InvalidByte { byte, offset }
                });
            }
        }
        return Err(crate::Error::InvalidPadding);
    }
    let z8 = std::arch::x86_64::_pext_u64(y8, 0x3f3f_3f3f_3f3f_3f3f);

    // Three-character tail (two output bytes); the fourth character is '='.
    let x3 = (input.as_ptr().add(168) as *const u32).read_unaligned();
    let y3 = ((*table.add((x3 & 255) as usize) as u64) << 16)
        | ((*table.add(((x3 >> 8) & 255) as usize) as u64) << 8)
        | (*table.add(((x3 >> 16) & 255) as usize) as u64);
    if y3 & 0x0080_8080 != 0 {
        for (i, &byte) in input[168..171].iter().enumerate() {
            let offset = i + 168;
            if *table.add(byte as usize) == 0xff {
                return Err(if byte == b'=' {
                    crate::Error::InvalidPadding
                } else {
                    crate::Error::InvalidByte { byte, offset }
                });
            }
        }
        return Err(crate::Error::InvalidPadding);
    }
    let z3 = std::arch::x86_64::_pext_u64(y3, 0x3f_3f3f) as u32;
    if z3 & 0x03 != 0 {
        return Err(crate::Error::InvalidPadding);
    }

    // SAFETY: output has 128 bytes; six new bytes start at 120 and the final
    // two at 126.
    let p = output.as_mut_ptr().add(120).cast::<u8>();
    p.write((z8 >> 40) as u8);
    p.add(1).write((z8 >> 32) as u8);
    p.add(2).write((z8 >> 24) as u8);
    p.add(3).write((z8 >> 16) as u8);
    p.add(4).write((z8 >> 8) as u8);
    p.add(5).write(z8 as u8);
    p.add(6).write((z3 >> 10) as u8);
    p.add(7).write((z3 >> 2) as u8);
    Ok(128)
}

/// Decode an arbitrary multiple-of-four prefix in four-block batches. Error
/// markers are accumulated across four blocks and checked once; valid input
/// dominates and the rare failure is located by re-checking the four vectors.
///
/// # Safety
/// AVX2 must be available. `output` must hold `input.len() / 4 * 3` bytes.
#[cfg(test)]
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

/// Decode a short, whitespace-free prefix whose length is a multiple of four.
/// The final partial 32-byte group is handled by one overlapping block, so the
/// constants and dispatch overhead are paid once even for 32-512 byte inputs.
///
/// # Safety
/// AVX2 must be available. `input.len()` must be a multiple of four and at
/// least 32, and `output` must hold `input.len() / 4 * 3` bytes.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn decode_any(
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
    // SAFETY: these arrays contain 16 bytes, exactly the size of the loads.
    let low_lut =
        unsafe { _mm256_broadcastsi128_si256(_mm_loadu_si128(low_table.as_ptr().cast())) };
    // SAFETY: the roll table contains 16 bytes and is loaded without alignment requirements.
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
            // which is either a later full block or the overlapping block, and
            // is overwritten there. The caller guarantees that output.
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

    let blocks = input.len() / 32;
    let mut block = 0;
    while block < blocks {
        let (values, bad) = translate_at!(block * 32);
        let mask = _mm256_movemask_epi8(bad) as u32;
        if mask != 0 {
            return Err(block * 32 + mask.trailing_zeros() as usize);
        }
        let rem = input.len() % 32;
        if block + 1 < blocks || rem >= 8 {
            store_at_next!(values, block * 24);
        } else {
            store_at!(values, block * 24);
        }
        block += 1;
    }
    let rem = input.len() % 32;
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

/// First non-alphabet byte, used only when the speculative assembly loop
/// detects an error. '=' is not in the bulk alphabet.
#[cfg(target_os = "linux")]
fn first_invalid(input: &[u8], alphabet: Alphabet) -> usize {
    input
        .iter()
        .position(|&byte| match alphabet {
            Alphabet::Standard => !(byte.is_ascii_alphanumeric() || byte == b'+' || byte == b'/'),
            Alphabet::UrlSafe => !(byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'),
        })
        .unwrap_or(0)
}

/// Decode complete blocks of 32 alphabet bytes to 24 bytes, validating in place.
///
/// # Safety
/// The processor must support AVX2; output must have room for
/// `input.len() / 32 * 24` bytes.
#[target_feature(enable = "avx2")]
pub(crate) unsafe fn decode(
    input: &[u8],
    output: &mut [std::mem::MaybeUninit<u8>],
    alphabet: Alphabet,
) -> Result<usize, usize> {
    let blocks = input.len() / 32;
    // Rule 1: construct the low-nibble membership bitmap for the selected
    // alphabet. High-nibble bitmaps intersect it to validate 32 characters
    // with two register shuffles; see THIRD-PARTY.md.
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
    // SAFETY: these arrays contain 16 bytes, exactly the size of the loads.
    let low_lut =
        unsafe { _mm256_broadcastsi128_si256(_mm_loadu_si128(low_table.as_ptr().cast())) };
    // SAFETY: the roll table contains 16 bytes and is loaded without alignment requirements.
    let roll_lut = unsafe { _mm256_broadcastsi128_si256(_mm_loadu_si128(roll.as_ptr().cast())) };
    let high_lut = _mm256_setr_epi8(
        1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 64, -128, 0, 0,
        0, 0, 0, 0, 0, 0,
    );
    let pair_mul = _mm256_set1_epi16(0x0140);
    let quad_mul = _mm256_set1_epi32(0x0001_1000);
    // Rule 3: valid inputs dominate. Validate two independent blocks and OR
    // their error vectors before branching once; locate the first bad block
    // only on the slow path. Loads remain strictly within the input slice.
    macro_rules! translate {
        ($block:expr) => {{
            // SAFETY: the caller of the macro ensures this is a complete block.
            let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add($block * 32).cast()) };
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
    macro_rules! pack_store {
        ($values:expr, $block:expr, $next:expr) => {{
            // Rule 1: two multiply-add stages repack four sextets into three
            // bytes, followed by a register shuffle and exact bounded stores.
            let pairs = _mm256_maddubs_epi16($values, pair_mul);
            let groups = _mm256_madd_epi16(pairs, quad_mul);
            let shuffle = _mm256_setr_epi8(
                2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 2, 1, 0, 6, 5, 4, 10, 9, 8,
                14, 13, 12, -1, -1, -1, -1,
            );
            let packed = _mm256_shuffle_epi8(groups, shuffle);
            let out = $block * 24;
            if $next {
                // SAFETY: the next output block exists, so both stores fit.
                // On success its store overwrites these four extra bytes;
                // on error the caller is required to discard partial output.
                unsafe {
                    _mm_storeu_si128(
                        output.as_mut_ptr().add(out).cast(),
                        _mm256_castsi256_si128(packed),
                    );
                    _mm_storeu_si128(
                        output.as_mut_ptr().add(out + 12).cast(),
                        _mm256_extracti128_si256::<1>(packed),
                    );
                }
            } else {
                let reordered =
                    _mm256_permutevar8x32_epi32(packed, _mm256_setr_epi32(0, 1, 2, 4, 2, 4, 5, 6));
                // SAFETY: these stores remain in the block and their overlap
                // contains identical bytes.
                unsafe {
                    _mm_storeu_si128(
                        output.as_mut_ptr().add(out).cast(),
                        _mm256_castsi256_si128(reordered),
                    );
                    _mm_storeu_si128(
                        output.as_mut_ptr().add(out + 8).cast(),
                        _mm256_extracti128_si256::<1>(reordered),
                    );
                }
            }
        }};
    }
    macro_rules! check_errors {
        ($e0:expr, $e1:expr, $block:expr) => {{
            let errors = _mm256_or_si256($e0, $e1);
            // Rule 3: one flag-setting test replaces mask extraction and a
            // separate scalar compare on the common path.
            if _mm256_testz_si256(errors, errors) == 0 {
                for (i, error) in [$e0, $e1].into_iter().enumerate() {
                    let mask = _mm256_movemask_epi8(error) as u32;
                    if mask != 0 {
                        return Err(($block + i) * 32 + mask.trailing_zeros() as usize);
                    }
                }
            }
        }};
    }
    let mut block = 0;
    // Large inputs take the hand-scheduled loop. At least one block stays in
    // Rust so the assembly's four-byte store spill lands in valid output. With
    // a non-multiple of four this moves up to three blocks from the Rust tail
    // into the assembly; the tail also takes the final partial-batch block.
    #[cfg(target_os = "linux")]
    if blocks >= 8 {
        // Leave at least one block for the tail so the assembly's four-byte
        // store spill still lands in valid output. With a non-multiple of four
        // this moves up to three blocks from the Rust tail into the assembly.
        let groups = (blocks - 1) / 4;
        let params = match alphabet {
            Alphabet::Standard => &STD_HASH,
            Alphabet::UrlSafe => &URL_HASH,
        };
        // SAFETY: AVX2 is guaranteed; groups*128 input bytes and groups*96
        // output bytes are in bounds, and four blocks remain for the tail.
        let status = unsafe {
            decode_hash4(
                input.as_ptr(),
                output.as_mut_ptr().cast(),
                groups,
                params as *const HashParams as *const u8,
            )
        };
        if status == usize::MAX - 1 {
            return Err(first_invalid(input, alphabet));
        }
        if status != usize::MAX {
            return Err(status);
        }
        block = groups * 4;
    }
    if blocks - block >= 2 {
        // Rule 3: validate the next pair while the current pair packs and
        // stores. The check stays ahead of those stores, so an invalid block
        // is never followed by output from later blocks.
        let (mut p0, e0) = translate!(block);
        let (mut p1, e1) = translate!(block + 1);
        check_errors!(e0, e1, block);
        macro_rules! pair_step {
            ($block:expr) => {{
                let (v0, e0) = translate!($block);
                let (v1, e1) = translate!($block + 1);
                check_errors!(e0, e1, $block);
                pack_store!(p0, $block - 2, true);
                pack_store!(p1, $block - 1, true);
                p0 = v0;
                p1 = v1;
            }};
        }
        block += 2;
        // Rule 2: stay ahead of the hardware streamer on inputs that do not
        // fit in L1. Splitting the prefetching prefix out of the loop removes
        // a bounds compare and branch from every remaining iteration.
        let prefetch_end = input.len().saturating_sub(2080) / 32;
        while block < prefetch_end && blocks - block >= 2 {
            // SAFETY: `block < prefetch_end` proves both prefetch lines are
            // inside the input allocation.
            unsafe {
                _mm_prefetch(input.as_ptr().add(block * 32 + 2048).cast(), _MM_HINT_T0);
            }
            pair_step!(block);
            block += 2;
        }
        while blocks - block >= 2 {
            pair_step!(block);
            block += 2;
        }
        pack_store!(p0, block - 2, true);
        pack_store!(p1, block - 1, false);
    }
    for block in block..blocks {
        // SAFETY: each iteration accesses exactly one complete 32-byte block;
        // AVX2 support is required by this function's contract.
        let bytes = unsafe { _mm256_loadu_si256(input.as_ptr().add(block * 32).cast()) };
        // Rule 1: map the four ASCII intervals to six-bit values in parallel.
        let high = _mm256_and_si256(_mm256_srli_epi16(bytes, 4), _mm256_set1_epi8(15));
        // Rule 3: the common case validates all 32 bytes in the vector pass;
        // only an invalid block goes back to scalar error identification.
        let valid = _mm256_and_si256(
            _mm256_shuffle_epi8(low_lut, bytes),
            _mm256_shuffle_epi8(high_lut, high),
        );
        let invalid = _mm256_movemask_epi8(_mm256_cmpeq_epi8(valid, _mm256_setzero_si256())) as u32;
        if invalid != 0 {
            return Err(block * 32 + invalid.trailing_zeros() as usize);
        }
        let index = _mm256_sub_epi8(high, _mm256_cmpgt_epi8(bytes, _mm256_set1_epi8(split)));
        let values = _mm256_add_epi8(bytes, _mm256_shuffle_epi8(roll_lut, index));
        // Rule 1: multiply-add repacks adjacent 6-bit values into 12- and
        // 24-bit groups; byte shuffle selects their big-endian bytes.
        let pairs = _mm256_maddubs_epi16(values, _mm256_set1_epi32(0x01400140));
        let groups = _mm256_madd_epi16(pairs, _mm256_set1_epi32(0x00011000));
        let shuffle = _mm256_setr_epi8(
            2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 2, 1, 0, 6, 5, 4, 10, 9, 8, 14,
            13, 12, -1, -1, -1, -1,
        );
        let packed = _mm256_shuffle_epi8(groups, shuffle);
        // Rule 1: reorder 32-bit lanes so two overlapping 16-byte stores
        // write precisely the 24 decoded bytes without stack copies.
        let reordered =
            _mm256_permutevar8x32_epi32(packed, _mm256_setr_epi32(0, 1, 2, 4, 2, 4, 5, 6));
        let out = block * 24;
        // SAFETY: both 16-byte stores lie within the 24-byte output block;
        // the overlapping eight bytes contain identical decoded data.
        unsafe {
            _mm_storeu_si128(
                output.as_mut_ptr().add(out).cast(),
                _mm256_castsi256_si128(reordered),
            );
            _mm_storeu_si128(
                output.as_mut_ptr().add(out + 8).cast(),
                _mm256_extracti128_si256::<1>(reordered),
            );
        }
    }
    Ok(blocks * 32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    #[test]
    fn whitespace_filter_matches_scalar_for_all_bytes_and_boundaries() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let mut input = [0u8; 192];
        for len in [0, 1, 31, 32, 33, 63, 64, 65, 127, 128, 129, 192] {
            for byte in 0..=255u8 {
                for (i, b) in input.iter_mut().enumerate() {
                    *b = if i % 7 == 0 { byte } else { (i * 73) as u8 };
                }
                for capacity in [0, 1, 31, 32, 33, 64, 127, 192] {
                    let mut storage = [MaybeUninit::new(0xa5u8); 208];
                    // SAFETY: AVX2 was detected and both disjoint slices are
                    // valid for their respective lengths.
                    let (consumed, written) =
                        unsafe { filter_whitespace(&input[..len], &mut storage[8..8 + capacity]) };
                    let expected: Vec<u8> = input[..consumed]
                        .iter()
                        .copied()
                        .filter(|b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
                        .collect();
                    assert_eq!(consumed, len.min(capacity));
                    assert_eq!(written, expected.len());
                    // SAFETY: the whole array was initialized before calling
                    // the filter, which only writes initialized bytes.
                    let bytes = unsafe {
                        std::slice::from_raw_parts(storage.as_ptr().cast::<u8>(), storage.len())
                    };
                    assert_eq!(&bytes[8..8 + written], expected);
                    assert!(bytes[..8].iter().all(|&b| b == 0xa5));
                    assert!(bytes[8 + capacity..].iter().all(|&b| b == 0xa5));
                }
            }
        }
    }

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

    #[cfg(target_os = "linux")]
    #[test]
    fn hash4_all_a() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let encoded = [b'A'; 128];
        // Four blocks produce 96 bytes; the last store spills four bytes, so
        // the test buffer is one block larger and only the valid prefix is
        // compared.
        let mut out = [0x55u8; 128];
        // SAFETY: AVX2 is available; four blocks produce 96 bytes.
        let status = unsafe {
            decode_hash4(
                encoded.as_ptr(),
                out.as_mut_ptr(),
                1,
                &STD_HASH as *const HashParams as *const u8,
            )
        };
        assert_eq!(status, usize::MAX);
        assert_eq!(&out[..96], &[0u8; 96]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn hash4_matches_reference() {
        use base64::Engine as _;
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        let decoded: Vec<u8> = (0..96).map(|i| (i * 37 + 11) as u8).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&decoded);
        let mut out = [0u8; 128];
        // SAFETY: AVX2 is available; four blocks produce 96 bytes.
        let status = unsafe {
            decode_hash4(
                encoded.as_ptr(),
                out.as_mut_ptr(),
                1,
                &STD_HASH as *const HashParams as *const u8,
            )
        };
        assert_eq!(status, usize::MAX);
        assert_eq!(&out[..96], &decoded[..]);
    }

    #[test]
    fn mulhi_matches_words() {
        if !std::is_x86_feature_detected!("avx2") {
            return;
        }
        // SAFETY: AVX2 was detected and the store writes exactly 32 bytes into
        // the 16-element u16 array.
        let out = unsafe {
            let a = _mm256_setr_epi16(
                0x1234, -1, -32767, 7, 0x4000, 0x3333, 0x1111, -21555, 0x1234, -1, -32767, 7,
                0x4000, 0x3333, 0x1111, -21555,
            );
            let b = _mm256_set1_epi32(0x04000040);
            let got = mulhi_epu16(a, b);
            let mut out = [0u16; 16];
            _mm256_storeu_si256(out.as_mut_ptr().cast(), got);
            out
        };
        for (i, &x) in out.iter().enumerate() {
            let m = if i % 2 == 0 { 0x40 } else { 0x400 };
            let input = [
                0x1234u16, 0xffff, 0x8001, 7, 0x4000, 0x3333, 0x1111, 0xabcd, 0x1234, 0xffff,
                0x8001, 7, 0x4000, 0x3333, 0x1111, 0xabcd,
            ][i];
            assert_eq!(x, ((input as u32 * m) >> 16) as u16, "word {i}");
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
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

#[cfg(all(test, target_os = "linux"))]
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

#[cfg(all(test, target_os = "linux"))]
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

#[cfg(all(test, target_os = "linux"))]
static STANDARD_ENCODE_PARAMS: EncodeParams = encode_params(false);
#[cfg(all(test, target_os = "linux"))]
static URL_ENCODE_PARAMS: EncodeParams = encode_params(true);

#[cfg(all(test, target_os = "linux"))]
unsafe extern "C" {
    fn yolow64_encode_load16_groups8(
        src: *const u8,
        dst: *mut u8,
        chunks: usize,
        params: *const u8,
    );
}

#[cfg(all(test, target_os = "linux"))]
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

#[cfg(all(test, target_os = "linux"))]
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
