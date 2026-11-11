use crate::Alphabet;
#[cfg(target_os = "linux")]
use std::arch::global_asm;
use std::arch::x86_64::*;
use std::mem::MaybeUninit;

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
pub(super) const STANDARD_MEMBERSHIP: [u8; 16] = membership(false);
pub(super) const URL_MEMBERSHIP: [u8; 16] = membership(true);

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
pub(super) struct HashParams {
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
pub(super) static STD_HASH: HashParams = hash_params(
    [1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0, 12],
    [0, 0, 0, 19, 4, 191, 191, 185, 185, 16, 0, 191, 0, 185, 0, 0],
    [5, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 6, 10, 10, 10, 6],
    [
        128, 128, 128, 207, 200, 184, 213, 166, 181, 134, 180, 128, 148, 128, 128, 128,
    ],
);

#[cfg(target_os = "linux")]
pub(super) static URL_HASH: HashParams = hash_params(
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
    pub(super) fn decode_hash4(
        src: *const u8,
        dst: *mut u8,
        groups: usize,
        params: *const u8,
    ) -> usize;
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

/// Decode exactly 12 padded base64 characters to 8 bytes in one call.
///
/// The caller checks that `input[11] == b'=' && input[10] != b'='`. The 11
/// payload characters are loaded into a 128-bit vector with `'A'` padding in
/// bytes `11..16`, validated and decoded in one SIMD pass without reading past
/// the 12-byte slice.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode12(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    _table: *const u8,
) -> Result<usize, crate::Error> {
    if output.len() < 8 {
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
    let low_lut = _mm_loadu_si128(low_table.as_ptr().cast());
    let roll_lut = _mm_loadu_si128(roll.as_ptr().cast());
    let high_lut = _mm_setr_epi8(1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0);

    let low64 = (input.as_ptr() as *const u64).read_unaligned();
    let tail24 = ((input.as_ptr().add(8) as *const u32).read_unaligned() & 0x00ff_ffff)
        | ((b'A' as u32) << 24);
    let high64 = 0x4141_4141_0000_0000u64 | (tail24 as u64);
    let bytes = _mm_set_epi64x(high64 as i64, low64 as i64);

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
    let shuffle = _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, -1, -1, -1, -1, -1, -1, -1);
    let packed = _mm_shuffle_epi8(groups, shuffle);
    if _mm_extract_epi8::<8>(packed) != 0 {
        return Err(crate::Error::InvalidPadding);
    }
    _mm_storel_epi64(output.as_mut_ptr().cast(), packed);
    Ok(8)
}

/// Decode 6 base64 characters (the prefix of an 8-character `==`-padded tail)
/// to a 32-bit big-endian word using BMI2 `pext`.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "bmi2")]
#[inline]
unsafe fn pext6_be(
    input: &[u8],
    base_offset: usize,
    table: *const u8,
) -> Result<u32, crate::Error> {
    let x = (input.as_ptr().add(base_offset) as *const u64).read_unaligned();
    let y = ((*table.add((x & 255) as usize) as u64) << 40)
        | ((*table.add(((x >> 8) & 255) as usize) as u64) << 32)
        | ((*table.add(((x >> 16) & 255) as usize) as u64) << 24)
        | ((*table.add(((x >> 24) & 255) as usize) as u64) << 16)
        | ((*table.add(((x >> 32) & 255) as usize) as u64) << 8)
        | (*table.add(((x >> 40) & 255) as usize) as u64);
    if y & 0x8080_8080_8080 != 0 {
        for (i, &byte) in input[base_offset..base_offset + 6].iter().enumerate() {
            let offset = base_offset + i;
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
    Ok(((z >> 4) as u32).to_be())
}

/// Decode exactly 24 padded base64 characters to 16 bytes in one call.
///
/// The first 16 characters (`0..16`) and the overlapping last 16 characters
/// (`8..24`, with trailing `==` replaced by `"AA"`) are combined into a single
/// 256-bit AVX2 vector and validated/decoded in one pass.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode24(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    _table: *const u8,
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
    let low_lut = _mm256_broadcastsi128_si256(_mm_loadu_si128(low_table.as_ptr().cast()));
    let roll_lut = _mm256_broadcastsi128_si256(_mm_loadu_si128(roll.as_ptr().cast()));
    let high_lut = _mm256_setr_epi8(
        1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 4, 8, 16, 32, 64, -128, 0, 0,
        0, 0, 0, 0, 0, 0,
    );
    let b_lo = _mm_loadu_si128(input.as_ptr().cast());
    let b_hi = _mm_insert_epi16::<7>(_mm_loadu_si128(input.as_ptr().add(8).cast()), 0x4141);
    let bytes = _mm256_set_m128i(b_hi, b_lo);

    let high = _mm256_and_si256(_mm256_srli_epi16(bytes, 4), _mm256_set1_epi8(15));
    let valid = _mm256_and_si256(
        _mm256_shuffle_epi8(low_lut, bytes),
        _mm256_shuffle_epi8(high_lut, high),
    );
    let bad = _mm256_cmpeq_epi8(valid, _mm256_setzero_si256());
    let mask = _mm256_movemask_epi8(bad) as u32;
    if mask != 0 {
        let lo_mask = mask & 0xffff;
        let offset = if lo_mask != 0 {
            lo_mask.trailing_zeros() as usize
        } else {
            8 + (mask >> 16).trailing_zeros() as usize
        };
        return Err(if input[offset] == b'=' {
            crate::Error::InvalidPadding
        } else {
            crate::Error::InvalidByte {
                byte: input[offset],
                offset,
            }
        });
    }
    let index = _mm256_sub_epi8(high, _mm256_cmpgt_epi8(bytes, _mm256_set1_epi8(split)));
    let values = _mm256_add_epi8(bytes, _mm256_shuffle_epi8(roll_lut, index));
    let pairs = _mm256_maddubs_epi16(values, _mm256_set1_epi32(0x0140_0140));
    let groups = _mm256_madd_epi16(pairs, _mm256_set1_epi32(0x0001_1000));
    let shuffle = _mm256_setr_epi8(
        2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 10, 9, 8, 14, 13, -1, -1, -1, -1,
        -1, -1, -1, -1, -1, -1, -1,
    );
    let packed = _mm256_shuffle_epi8(groups, shuffle);
    let packed_hi = _mm256_extracti128_si256::<1>(packed);
    if _mm_extract_epi8::<4>(packed_hi) != 0 {
        return Err(crate::Error::InvalidPadding);
    }
    let dst = output.as_mut_ptr();
    _mm_storeu_si128(dst.cast(), _mm256_castsi256_si128(packed));
    (dst.add(12) as *mut i32).write_unaligned(_mm_cvtsi128_si32(packed_hi));
    Ok(16)
}

/// Decode exactly 44 padded base64 characters to 32 bytes in one call using
/// one 256-bit AVX2 block and one overlapping 128-bit block.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode44(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    _table: *const u8,
) -> Result<usize, crate::Error> {
    if output.len() < 32 {
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
    let low_lut128 = _mm_loadu_si128(low_table.as_ptr().cast());
    let roll_lut128 = _mm_loadu_si128(roll.as_ptr().cast());
    let high_lut128 = _mm_setr_epi8(1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0);
    let low_lut = _mm256_broadcastsi128_si256(low_lut128);
    let roll_lut = _mm256_broadcastsi128_si256(roll_lut128);
    let high_lut = _mm256_broadcastsi128_si256(high_lut128);

    let b0 = _mm256_loadu_si256(input.as_ptr().cast());
    let b1 = _mm_insert_epi8::<15>(
        _mm_loadu_si128(input.as_ptr().add(28).cast()),
        b'A' as i32,
    );

    let h0 = _mm256_and_si256(_mm256_srli_epi16(b0, 4), _mm256_set1_epi8(15));
    let h1 = _mm_and_si128(_mm_srli_epi16::<4>(b1), _mm_set1_epi8(15));
    let v0 = _mm256_and_si256(
        _mm256_shuffle_epi8(low_lut, b0),
        _mm256_shuffle_epi8(high_lut, h0),
    );
    let v1 = _mm_and_si128(
        _mm_shuffle_epi8(low_lut128, b1),
        _mm_shuffle_epi8(high_lut128, h1),
    );
    let bad0 = _mm256_cmpeq_epi8(v0, _mm256_setzero_si256());
    let bad1 = _mm_cmpeq_epi8(v1, _mm_setzero_si128());
    let bad_all = _mm256_or_si256(bad0, _mm256_zextsi128_si256(bad1));
    if _mm256_testz_si256(bad_all, bad_all) == 0 {
        let mask0 = _mm256_movemask_epi8(bad0) as u32;
        if mask0 != 0 {
            let offset = mask0.trailing_zeros() as usize;
            return Err(if input[offset] == b'=' {
                crate::Error::InvalidPadding
            } else {
                crate::Error::InvalidByte {
                    byte: input[offset],
                    offset,
                }
            });
        }
        let mask1 = _mm_movemask_epi8(bad1) as u32;
        let offset = 28 + mask1.trailing_zeros() as usize;
        return Err(if input[offset] == b'=' {
            crate::Error::InvalidPadding
        } else {
            crate::Error::InvalidByte {
                byte: input[offset],
                offset,
            }
        });
    }

    let idx0 = _mm256_sub_epi8(h0, _mm256_cmpgt_epi8(b0, _mm256_set1_epi8(split)));
    let val0 = _mm256_add_epi8(b0, _mm256_shuffle_epi8(roll_lut, idx0));
    let g0 = _mm256_madd_epi16(
        _mm256_maddubs_epi16(val0, _mm256_set1_epi32(0x0140_0140)),
        _mm256_set1_epi32(0x0001_1000),
    );
    let shuf0 = _mm256_setr_epi8(
        2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, 2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13,
        12, -1, -1, -1, -1,
    );
    let p0 = _mm256_shuffle_epi8(g0, shuf0);

    let idx1 = _mm_sub_epi8(h1, _mm_cmpgt_epi8(b1, _mm_set1_epi8(split)));
    let val1 = _mm_add_epi8(b1, _mm_shuffle_epi8(roll_lut128, idx1));
    let g1 = _mm_madd_epi16(
        _mm_maddubs_epi16(val1, _mm_set1_epi32(0x0140_0140)),
        _mm_set1_epi32(0x0001_1000),
    );
    let shuf1 = _mm_setr_epi8(6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1, -1, -1, -1);
    let p1 = _mm_shuffle_epi8(g1, shuf1);
    if _mm_extract_epi8::<8>(p1) != 0 {
        return Err(crate::Error::InvalidPadding);
    }

    let dst = output.as_mut_ptr();
    _mm_storeu_si128(dst.cast(), _mm256_castsi256_si128(p0));
    _mm_storeu_si128(dst.add(12).cast(), _mm256_extracti128_si256::<1>(p0));
    _mm_storel_epi64(dst.add(24).cast(), p1);
    Ok(32)
}

/// Decode exactly 88 padded base64 characters to 64 bytes in one call.
///
/// The first 64 characters use two 256-bit AVX2 blocks; the next 16 use a
/// 128-bit block sharing the same LUT registers; the last 6 use BMI2 `pext`.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode88(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    table: *const u8,
) -> Result<usize, crate::Error> {
    if output.len() < 64 {
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
    let low_lut128 = _mm_loadu_si128(low_table.as_ptr().cast());
    let roll_lut128 = _mm_loadu_si128(roll.as_ptr().cast());
    let high_lut128 = _mm_setr_epi8(1, 2, 4, 8, 16, 32, 64, -128, 0, 0, 0, 0, 0, 0, 0, 0);
    let low_lut = _mm256_broadcastsi128_si256(low_lut128);
    let roll_lut = _mm256_broadcastsi128_si256(roll_lut128);
    let high_lut = _mm256_broadcastsi128_si256(high_lut128);
    let split256 = _mm256_set1_epi8(split);
    let split128 = _mm_set1_epi8(split);
    let mask15_256 = _mm256_set1_epi8(15);
    let mask15_128 = _mm_set1_epi8(15);

    let b0 = _mm256_loadu_si256(input.as_ptr().cast());
    let b1 = _mm256_loadu_si256(input.as_ptr().add(32).cast());
    let b2 = _mm_loadu_si128(input.as_ptr().add(64).cast());

    let h0 = _mm256_and_si256(_mm256_srli_epi16(b0, 4), mask15_256);
    let h1 = _mm256_and_si256(_mm256_srli_epi16(b1, 4), mask15_256);
    let h2 = _mm_and_si128(_mm_srli_epi16::<4>(b2), mask15_128);

    let v0 = _mm256_and_si256(
        _mm256_shuffle_epi8(low_lut, b0),
        _mm256_shuffle_epi8(high_lut, h0),
    );
    let v1 = _mm256_and_si256(
        _mm256_shuffle_epi8(low_lut, b1),
        _mm256_shuffle_epi8(high_lut, h1),
    );
    let v2 = _mm_and_si128(
        _mm_shuffle_epi8(low_lut128, b2),
        _mm_shuffle_epi8(high_lut128, h2),
    );

    let bad0 = _mm256_cmpeq_epi8(v0, _mm256_setzero_si256());
    let bad1 = _mm256_cmpeq_epi8(v1, _mm256_setzero_si256());
    let bad2 = _mm_cmpeq_epi8(v2, _mm_setzero_si128());
    let bad_all = _mm256_or_si256(_mm256_or_si256(bad0, bad1), _mm256_zextsi128_si256(bad2));
    if _mm256_testz_si256(bad_all, bad_all) == 0 {
        for (base, mask) in [
            (0usize, _mm256_movemask_epi8(bad0) as u32),
            (32, _mm256_movemask_epi8(bad1) as u32),
            (64, _mm_movemask_epi8(bad2) as u32),
        ] {
            if mask != 0 {
                let offset = base + mask.trailing_zeros() as usize;
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
    }

    let pair_mul256 = _mm256_set1_epi32(0x0140_0140);
    let quad_mul256 = _mm256_set1_epi32(0x0001_1000);
    let shuffle128 = _mm_setr_epi8(2, 1, 0, 6, 5, 4, 10, 9, 8, 14, 13, 12, -1, -1, -1, -1);
    let shuffle256 = _mm256_broadcastsi128_si256(shuffle128);

    let idx0 = _mm256_sub_epi8(h0, _mm256_cmpgt_epi8(b0, split256));
    let val0 = _mm256_add_epi8(b0, _mm256_shuffle_epi8(roll_lut, idx0));
    let p0 = _mm256_shuffle_epi8(
        _mm256_madd_epi16(_mm256_maddubs_epi16(val0, pair_mul256), quad_mul256),
        shuffle256,
    );

    let idx1 = _mm256_sub_epi8(h1, _mm256_cmpgt_epi8(b1, split256));
    let val1 = _mm256_add_epi8(b1, _mm256_shuffle_epi8(roll_lut, idx1));
    let p1 = _mm256_shuffle_epi8(
        _mm256_madd_epi16(_mm256_maddubs_epi16(val1, pair_mul256), quad_mul256),
        shuffle256,
    );

    let idx2 = _mm_sub_epi8(h2, _mm_cmpgt_epi8(b2, split128));
    let val2 = _mm_add_epi8(b2, _mm_shuffle_epi8(roll_lut128, idx2));
    let p2 = _mm_shuffle_epi8(
        _mm_madd_epi16(
            _mm_maddubs_epi16(val2, _mm_set1_epi32(0x0140_0140)),
            _mm_set1_epi32(0x0001_1000),
        ),
        shuffle128,
    );

    let dst = output.as_mut_ptr();
    _mm_storeu_si128(dst.cast(), _mm256_castsi256_si128(p0));
    _mm_storeu_si128(dst.add(12).cast(), _mm256_extracti128_si256::<1>(p0));
    _mm_storeu_si128(dst.add(24).cast(), _mm256_castsi256_si128(p1));
    _mm_storeu_si128(dst.add(36).cast(), _mm256_extracti128_si256::<1>(p1));
    _mm_storeu_si128(dst.add(48).cast(), p2);

    let word = pext6_be(input, 80, table)?;
    (dst.add(60) as *mut u32).write_unaligned(word);
    Ok(64)
}

/// Decode `groups * 128 + 44` padded base64 characters (`groups >= 1`) to
/// `groups * 96 + 32` bytes in one call.
#[cfg(target_arch = "x86_64")]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode_hash4_tail44(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    table: *const u8,
) -> Result<usize, crate::Error> {
    let groups = input.len() / 128;
    let head_in = groups * 128;
    let head_out = groups * 96;
    let total_out = head_out + 32;
    if output.len() < total_out {
        return Err(crate::Error::OutputTooSmall);
    }
    #[cfg(target_os = "linux")]
    {
        let params = match alphabet {
            Alphabet::Standard => &STD_HASH as *const HashParams as *const u8,
            Alphabet::UrlSafe => &URL_HASH as *const HashParams as *const u8,
        };
        let status = decode_hash4(input.as_ptr(), output.as_mut_ptr().cast(), groups, params);
        if status != usize::MAX {
            let offset = first_invalid(&input[..head_in], alphabet);
            let byte = input[offset];
            return Err(if byte == b'=' {
                crate::Error::InvalidPadding
            } else {
                crate::Error::InvalidByte { byte, offset }
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    {
        if let Err(offset) = decode(&input[..head_in], &mut output[..head_out], alphabet) {
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
    decode44(
        &input[head_in..],
        &mut output[head_out..total_out],
        alphabet,
        table,
    )
    .map_err(|err| match err {
        crate::Error::InvalidByte { byte, offset } => crate::Error::InvalidByte {
            byte,
            offset: head_in + offset,
        },
        other => other,
    })?;
    Ok(total_out)
}

/// Decode `groups * 128 + 88` padded base64 characters (`groups >= 1`) to
/// `groups * 96 + 64` bytes in one call.
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
#[allow(unsafe_op_in_unsafe_fn)]
#[target_feature(enable = "avx2,bmi2")]
pub(crate) unsafe fn decode_hash4_tail88(
    input: &[u8],
    output: &mut [MaybeUninit<u8>],
    alphabet: Alphabet,
    table: *const u8,
) -> Result<usize, crate::Error> {
    let groups = input.len() / 128;
    let head_in = groups * 128;
    let head_out = groups * 96;
    let total_out = head_out + 64;
    if output.len() < total_out {
        return Err(crate::Error::OutputTooSmall);
    }
    let params = match alphabet {
        Alphabet::Standard => &STD_HASH as *const HashParams as *const u8,
        Alphabet::UrlSafe => &URL_HASH as *const HashParams as *const u8,
    };
    let status = decode_hash4(input.as_ptr(), output.as_mut_ptr().cast(), groups, params);
    if status != usize::MAX {
        let offset = first_invalid(&input[..head_in], alphabet);
        let byte = input[offset];
        return Err(if byte == b'=' {
            crate::Error::InvalidPadding
        } else {
            crate::Error::InvalidByte { byte, offset }
        });
    }
    decode88(
        &input[head_in..],
        &mut output[head_out..total_out],
        alphabet,
        table,
    )
    .map_err(|err| match err {
        crate::Error::InvalidByte { byte, offset } => crate::Error::InvalidByte {
            byte,
            offset: head_in + offset,
        },
        other => other,
    })?;
    Ok(total_out)
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
