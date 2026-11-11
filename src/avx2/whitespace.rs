//! AVX2 whitespace detection and filtering kernels.

use std::arch::x86_64::*;
use std::mem::MaybeUninit;

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
