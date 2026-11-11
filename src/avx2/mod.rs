mod decode;
mod encode;
#[cfg(test)]
pub(crate) mod lab;
mod whitespace;

#[cfg(target_os = "linux")]
pub(crate) use decode::decode_hash4_tail88;
pub(crate) use decode::{
    decode, decode_any, decode_hash4_tail44, decode12, decode16, decode24, decode44, decode88,
};
pub(crate) use encode::{
    encode, encode_8, encode_12, encode_16, encode_32, encode_load16, encode_unroll8_split,
};
pub(crate) use whitespace::{filter_whitespace, first_special};

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use super::decode::{HashParams, STD_HASH, decode_hash4};
    use super::encode::mulhi_epu16;
    use std::arch::x86_64::*;
    use std::mem::MaybeUninit;

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
