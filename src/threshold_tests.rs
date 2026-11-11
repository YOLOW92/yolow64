//! Ignored exploratory crossover test; run under the shared benchmark lock.
use crate::{Alphabet, STANDARD};
use std::{hint::black_box, time::Instant};

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn decode_crossover() {
    if !crate::cpu::avx2_available() {
        return;
    }
    println!("encoded_bytes,scalar_ns,short_ns,simd_ns");
    for size in [32usize, 48, 64, 96, 128, 192, 256, 512, 1024, 4096] {
        let bytes = vec![b'A'; size];
        let mut dst = vec![0u8; size / 4 * 3];
        let iterations = 200_000;
        let start = Instant::now();
        for _ in 0..iterations {
            STANDARD
                .decode_scalar(
                    black_box(&bytes),
                    black_box(crate::maybe_uninit_slice(&mut dst)),
                )
                .unwrap();
        }
        let scalar = start.elapsed().as_nanos() as f64 / iterations as f64;
        let start = Instant::now();
        for _ in 0..iterations {
            STANDARD
                .decode_short(
                    black_box(&bytes),
                    black_box(crate::maybe_uninit_slice(&mut dst)),
                )
                .unwrap();
        }
        let short = start.elapsed().as_nanos() as f64 / iterations as f64;
        let start = Instant::now();
        for _ in 0..iterations {
            // SAFETY: runtime detection succeeded; input is a whole number of
            // 32-byte valid blocks and output has 24 bytes per block.
            unsafe {
                crate::avx2::decode(
                    black_box(&bytes),
                    black_box(crate::maybe_uninit_slice(&mut dst)),
                    Alphabet::Standard,
                )
            }
            .unwrap();
        }
        let simd = start.elapsed().as_nanos() as f64 / iterations as f64;
        println!("{size},{scalar:.1},{short:.1},{simd:.1}");
    }
}

/// Focused short-decode timing lab.
#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn decode_short_lab() {
    use base64::Engine as _;
    use std::mem::MaybeUninit;

    if !crate::cpu::avx2_available() {
        return;
    }
    fn measure(mut f: impl FnMut(), iters: usize) -> f64 {
        let mut values = Vec::with_capacity(21);
        for _ in 0..21 {
            let start = Instant::now();
            for _ in 0..iters {
                f();
            }
            values.push(start.elapsed().as_secs_f64() * 1e9 / iters as f64);
        }
        values.sort_by(f64::total_cmp);
        values[10]
    }
    println!("payload,encoded,public_ns,direct_ns,direct_kind");
    for &payload in &[8usize, 12, 16, 24, 32, 48, 64, 96, 128] {
        let raw: Vec<u8> = (0..payload).map(|i| (i * 29 + 7) as u8).collect();
        let enc = base64::engine::general_purpose::STANDARD
            .encode(&raw)
            .into_bytes();
        let mut out = vec![0u8; payload];
        let mut uninit = vec![MaybeUninit::<u8>::uninit(); payload];
        let iters = 200_000;
        let public = measure(
            || {
                black_box(
                    STANDARD
                        .decode(black_box(&enc), black_box(&mut out))
                        .unwrap(),
                );
            },
            iters,
        );
        let (direct, kind) = match enc.len() {
            12 => (
                measure(
                    || {
                        black_box(
                            STANDARD
                                .decode_short(black_box(&enc), black_box(&mut uninit))
                                .unwrap(),
                        );
                    },
                    iters,
                ),
                "short",
            ),
            16 => (
                measure(
                    || {
                        let head: &[u8; 16] = enc[..16].try_into().unwrap();
                        let result = unsafe {
                            crate::avx2::decode16(
                                black_box(head),
                                black_box(&mut uninit[..12]),
                                Alphabet::Standard,
                            )
                        };
                        black_box(result).unwrap();
                    },
                    iters,
                ),
                "decode16",
            ),
            24 => (
                measure(
                    || {
                        let head: &[u8; 16] = enc[..16].try_into().unwrap();
                        unsafe {
                            crate::avx2::decode16(
                                black_box(head),
                                black_box(&mut uninit[..12]),
                                Alphabet::Standard,
                            )
                            .unwrap();
                        }
                        black_box(
                            STANDARD
                                .decode_short(black_box(&enc[16..]), black_box(&mut uninit[12..]))
                                .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode16+short",
            ),
            32 => (
                measure(
                    || {
                        black_box(
                            unsafe {
                                crate::avx2::decode(
                                    black_box(&enc),
                                    black_box(&mut uninit[..]),
                                    Alphabet::Standard,
                                )
                            }
                            .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode32",
            ),
            44 => (
                measure(
                    || {
                        unsafe {
                            crate::avx2::decode(
                                black_box(&enc[..32]),
                                black_box(&mut uninit[..24]),
                                Alphabet::Standard,
                            )
                            .unwrap();
                        }
                        black_box(
                            STANDARD
                                .decode_short(black_box(&enc[32..]), black_box(&mut uninit[24..]))
                                .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode32+short",
            ),
            64 => (
                measure(
                    || {
                        black_box(
                            unsafe {
                                crate::avx2::decode(
                                    black_box(&enc),
                                    black_box(&mut uninit[..]),
                                    Alphabet::Standard,
                                )
                            }
                            .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode64",
            ),
            88 => (
                measure(
                    || {
                        unsafe {
                            crate::avx2::decode(
                                black_box(&enc[..64]),
                                black_box(&mut uninit[..48]),
                                Alphabet::Standard,
                            )
                            .unwrap();
                        }
                        black_box(
                            STANDARD
                                .decode_short(black_box(&enc[64..]), black_box(&mut uninit[48..]))
                                .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode64+short",
            ),
            128 => (
                measure(
                    || {
                        black_box(
                            unsafe {
                                crate::avx2::decode(
                                    black_box(&enc),
                                    black_box(&mut uninit[..]),
                                    Alphabet::Standard,
                                )
                            }
                            .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode128",
            ),
            172 => (
                measure(
                    || {
                        unsafe {
                            crate::avx2::decode(
                                black_box(&enc[..160]),
                                black_box(&mut uninit[..120]),
                                Alphabet::Standard,
                            )
                            .unwrap();
                        }
                        black_box(
                            STANDARD
                                .decode_short(black_box(&enc[160..]), black_box(&mut uninit[120..]))
                                .unwrap(),
                        );
                    },
                    iters,
                ),
                "decode160+short",
            ),
            _ => (f64::NAN, "none"),
        };
        assert_eq!(
            crate::STANDARD
                .decode(&enc, &mut vec![0u8; payload])
                .unwrap(),
            payload
        );
        println!("{payload},{},{public:.2},{direct:.2},{kind}", enc.len());
    }
}

/// Pext-based short-tail laboratory.
#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn pext_lab() {
    use base64::Engine as _;
    use std::mem::MaybeUninit;

    if !std::is_x86_feature_detected!("bmi2") || !crate::cpu::avx2_available() {
        return;
    }
    const fn table() -> [u8; 256] {
        let mut t = [0x80u8; 256];
        let mut b = 0usize;
        while b < 256 {
            t[b] = if b >= b'A' as usize && b <= b'Z' as usize {
                (b - b'A' as usize) as u8
            } else if b >= b'a' as usize && b <= b'z' as usize {
                (b - b'a' as usize + 26) as u8
            } else if b >= b'0' as usize && b <= b'9' as usize {
                (b - b'0' as usize + 52) as u8
            } else if b == b'+' as usize {
                62
            } else if b == b'/' as usize {
                63
            } else {
                0x80
            };
            b += 1;
        }
        t
    }
    static TABLE: [u8; 256] = table();
    #[allow(unsafe_op_in_unsafe_fn)]
    #[target_feature(enable = "bmi2")]
    unsafe fn pext6(p: *const u8) -> u64 {
        let t = TABLE.as_ptr();
        let x = (p as *const u64).read_unaligned();
        let y = ((*t.add((x & 255) as usize) as u64) << 40)
            | ((*t.add(((x >> 8) & 255) as usize) as u64) << 32)
            | ((*t.add(((x >> 16) & 255) as usize) as u64) << 24)
            | ((*t.add(((x >> 24) & 255) as usize) as u64) << 16)
            | ((*t.add(((x >> 32) & 255) as usize) as u64) << 8)
            | (*t.add(((x >> 40) & 255) as usize) as u64);
        if y & 0x8080_8080_8080 != 0 {
            return u64::MAX;
        }
        std::arch::x86_64::_pext_u64(y, 0x3f3f_3f3f_3f3f)
    }
    #[allow(unsafe_op_in_unsafe_fn)]
    #[target_feature(enable = "bmi2")]
    unsafe fn pext8(p: *const u8) -> u64 {
        let t = TABLE.as_ptr();
        let x = (p as *const u64).read_unaligned();
        let y = ((*t.add((x & 255) as usize) as u64) << 56)
            | ((*t.add(((x >> 8) & 255) as usize) as u64) << 48)
            | ((*t.add(((x >> 16) & 255) as usize) as u64) << 40)
            | ((*t.add(((x >> 24) & 255) as usize) as u64) << 32)
            | ((*t.add(((x >> 32) & 255) as usize) as u64) << 24)
            | ((*t.add(((x >> 40) & 255) as usize) as u64) << 16)
            | ((*t.add(((x >> 48) & 255) as usize) as u64) << 8)
            | (*t.add(((x >> 56) & 255) as usize) as u64);
        if y & 0x8080_8080_8080_8080 != 0 {
            return u64::MAX;
        }
        std::arch::x86_64::_pext_u64(y, 0x3f3f_3f3f_3f3f_3f3f)
    }
    #[allow(unsafe_op_in_unsafe_fn)]
    #[target_feature(enable = "bmi2")]
    unsafe fn pext4(p: *const u8) -> u32 {
        let t = TABLE.as_ptr();
        let x = (p as *const u32).read_unaligned();
        let y = ((*t.add((x & 255) as usize) as u64) << 16)
            | ((*t.add(((x >> 8) & 255) as usize) as u64) << 8)
            | (*t.add(((x >> 16) & 255) as usize) as u64);
        if y & 0x0080_8080 != 0 {
            return u32::MAX;
        }
        std::arch::x86_64::_pext_u64(y, 0x3f_3f3f) as u32
    }
    fn measure(mut f: impl FnMut(), iters: usize) -> f64 {
        let mut values = Vec::with_capacity(21);
        for _ in 0..21 {
            let start = Instant::now();
            for _ in 0..iters {
                f();
            }
            values.push(start.elapsed().as_secs_f64() * 1e9 / iters as f64);
        }
        values.sort_by(f64::total_cmp);
        values[10]
    }
    for &payload in &[16usize, 32] {
        let raw: Vec<u8> = (0..payload).map(|i| (i * 29 + 7) as u8).collect();
        let enc = base64::engine::general_purpose::STANDARD
            .encode(&raw)
            .into_bytes();
        let mut out = vec![0u8; payload];
        let mut uninit = vec![MaybeUninit::<u8>::uninit(); payload];
        let iters = 500_000;
        let public = measure(
            || {
                black_box(
                    STANDARD
                        .decode(black_box(&enc), black_box(&mut out))
                        .unwrap(),
                );
            },
            iters,
        );
        let pext = measure(
            || {
                if payload == 16 {
                    let head: &[u8; 16] = enc[..16].try_into().unwrap();
                    unsafe {
                        crate::avx2::decode16(
                            black_box(head),
                            black_box(&mut uninit[..12]),
                            Alphabet::Standard,
                        )
                        .unwrap();
                        let z = pext6(black_box(enc.as_ptr().add(16)));
                        assert_ne!(z, u64::MAX);
                        let p = uninit.as_mut_ptr().add(12).cast::<u8>();
                        p.write((z >> 28) as u8);
                        p.add(1).write((z >> 20) as u8);
                        p.add(2).write((z >> 12) as u8);
                        p.add(3).write((z >> 4) as u8);
                    }
                } else {
                    unsafe {
                        crate::avx2::decode(
                            black_box(&enc[..32]),
                            black_box(&mut uninit[..24]),
                            Alphabet::Standard,
                        )
                        .unwrap();
                        let z = pext8(black_box(enc.as_ptr().add(32)));
                        assert_ne!(z, u64::MAX);
                        let p = uninit.as_mut_ptr().add(24).cast::<u8>();
                        p.write((z >> 40) as u8);
                        p.add(1).write((z >> 32) as u8);
                        p.add(2).write((z >> 24) as u8);
                        p.add(3).write((z >> 16) as u8);
                        p.add(4).write((z >> 8) as u8);
                        p.add(5).write(z as u8);
                        let w = pext4(black_box(enc.as_ptr().add(40)));
                        assert_ne!(w, u32::MAX);
                        p.add(6).write((w >> 10) as u8);
                        p.add(7).write((w >> 2) as u8);
                    }
                }
            },
            iters,
        );
        println!("payload {payload}: public {public:.2}, pext-tail {pext:.2}");
        let mut check = vec![0u8; payload];
        unsafe {
            if payload == 16 {
                let head: &[u8; 16] = enc[..16].try_into().unwrap();
                crate::avx2::decode16(head, &mut uninit[..12], Alphabet::Standard).unwrap();
                let z = pext6(enc.as_ptr().add(16));
                let p = uninit.as_mut_ptr().add(12).cast::<u8>();
                p.write((z >> 28) as u8);
                p.add(1).write((z >> 20) as u8);
                p.add(2).write((z >> 12) as u8);
                p.add(3).write((z >> 4) as u8);
                std::ptr::copy_nonoverlapping(uninit.as_ptr().cast::<u8>(), check.as_mut_ptr(), 16);
            } else {
                crate::avx2::decode(&enc[..32], &mut uninit[..24], Alphabet::Standard).unwrap();
                let z = pext8(enc.as_ptr().add(32));
                let p = uninit.as_mut_ptr().add(24).cast::<u8>();
                p.write((z >> 40) as u8);
                p.add(1).write((z >> 32) as u8);
                p.add(2).write((z >> 24) as u8);
                p.add(3).write((z >> 16) as u8);
                p.add(4).write((z >> 8) as u8);
                p.add(5).write(z as u8);
                let w = pext4(enc.as_ptr().add(40));
                p.add(6).write((w >> 10) as u8);
                p.add(7).write((w >> 2) as u8);
                std::ptr::copy_nonoverlapping(uninit.as_ptr().cast::<u8>(), check.as_mut_ptr(), 32);
            }
        }
        assert_eq!(check, raw, "payload {payload}");
    }
}

/// Exercise the no-BMI2 fallback on a CPU that supports BMI2. The override is
/// thread-local so parallel tests are unaffected.
#[test]
#[cfg(target_arch = "x86_64")]
fn bmi2_fallback_short_paths() {
    use base64::Engine as _;

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::cpu::FORCE_NO_BMI2.with(|forced| forced.set(false));
        }
    }
    crate::cpu::FORCE_NO_BMI2.with(|forced| forced.set(true));
    let _reset = Reset;

    for (alphabet, reference) in [
        (
            Alphabet::Standard,
            &base64::engine::general_purpose::STANDARD,
        ),
        (
            Alphabet::UrlSafe,
            &base64::engine::general_purpose::URL_SAFE,
        ),
    ] {
        let codec = crate::Codec {
            alphabet,
            padding: true,
            ignore_whitespace: false,
        };
        for &payload in &[8usize, 12, 16, 24, 32, 48, 64, 96, 128, 8192, 16384] {
            let raw: Vec<u8> = (0..payload).map(|i| (i * 29 + 7) as u8).collect();
            let encoded = reference.encode(&raw).into_bytes();
            let mut output = vec![0u8; payload];
            let written = codec.decode(&encoded, &mut output).unwrap();
            assert_eq!(written, payload);
            assert_eq!(output, raw, "fallback mismatch for {alphabet:?} {payload}");
            let mut bad = encoded.clone();
            bad[0] = b'!';
            assert!(codec.decode(&bad, &mut output).is_err());
        }
    }
}

/// Exercise the no-AVX2 scalar fallback on a CPU that supports AVX2. The
/// override is thread-local so parallel tests are unaffected.
#[test]
#[cfg(target_arch = "x86_64")]
fn no_avx2_fallback_paths() {
    use base64::Engine as _;

    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            crate::cpu::FORCE_NO_AVX2.with(|forced| forced.set(false));
        }
    }
    crate::cpu::FORCE_NO_AVX2.with(|forced| forced.set(true));
    let _reset = Reset;

    assert!(!crate::cpu::avx2_available());
    for (alphabet, padding, reference) in [
        (
            Alphabet::Standard,
            true,
            &base64::engine::general_purpose::STANDARD,
        ),
        (
            Alphabet::Standard,
            false,
            &base64::engine::general_purpose::STANDARD_NO_PAD,
        ),
        (
            Alphabet::UrlSafe,
            true,
            &base64::engine::general_purpose::URL_SAFE,
        ),
        (
            Alphabet::UrlSafe,
            false,
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ),
    ] {
        for ignore_whitespace in [false, true] {
            let codec = crate::Codec {
                alphabet,
                padding,
                ignore_whitespace,
            };
            for &payload in &[0usize, 1, 2, 3, 8, 12, 16, 24, 32, 48, 64, 128, 4097] {
                let raw: Vec<u8> = (0..payload).map(|i| (i * 29 + 7) as u8).collect();
                let expected = reference.encode(&raw);
                let encoded = codec.encode_to_string(&raw).unwrap();
                assert_eq!(encoded, expected, "encode mismatch for {codec:?} {payload}");

                let mut wire = encoded.as_bytes().to_vec();
                if ignore_whitespace && !wire.is_empty() {
                    let mid = wire.len() / 2;
                    wire.splice(mid..mid, *b" \r\n\t");
                }
                let decoded = codec.decode_to_vec(&wire).unwrap();
                assert_eq!(decoded, raw, "decode mismatch for {codec:?} {payload}");

                let mut stream = crate::Decoder::new(codec);
                let mut stream_out = Vec::new();
                let split = wire.len() / 2;
                stream.feed(&wire[..split], &mut stream_out).unwrap();
                stream.feed(&wire[split..], &mut stream_out).unwrap();
                stream.finish(&mut stream_out).unwrap();
                assert_eq!(stream_out, raw, "stream mismatch for {codec:?} {payload}");
            }
        }
    }
}

/// Exploratory decode-variant comparison, run with:
/// `cargo test --release decode_variant_lab -- --ignored --nocapture`
#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn decode_variant_lab() {
    use std::mem::MaybeUninit;
    let sizes = [
        32usize, 64, 96, 128, 256, 512, 640, 768, 896, 1024, 1280, 1536, 2048, 3072, 4096, 8192,
        65536,
    ];
    let rounds = 21usize;
    println!("size,public_ns,any_ns,batch4_ns,main_ns,batch4_vs_any_ns,batch4_vs_any_pct");
    for &size in &sizes {
        assert_eq!(size % 32, 0);
        let input = vec![b'A'; size];
        let decoded = size / 4 * 3;
        let mut out = vec![0u8; decoded];
        let mut any_out = vec![MaybeUninit::<u8>::uninit(); decoded];
        let mut batch_out = vec![MaybeUninit::<u8>::uninit(); decoded];
        let mut main_out = vec![MaybeUninit::<u8>::uninit(); decoded];
        let codec = crate::Codec {
            padding: false,
            ..crate::STANDARD
        };
        let iters = (4 * 1024 * 1024 / size).clamp(8, 50_000);

        let mut samples = [[0f64; 4]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..4 {
                let which = (step + round) % 4;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(
                                codec
                                    .decode(black_box(&input), black_box(&mut out))
                                    .unwrap(),
                            );
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::decode_any(
                                    black_box(&input),
                                    black_box(&mut any_out),
                                    crate::Alphabet::Standard,
                                )
                                .unwrap()
                            });
                        }
                        2 => {
                            black_box(unsafe {
                                crate::avx2::lab::decode_batch4(
                                    black_box(&input),
                                    black_box(&mut batch_out),
                                    crate::Alphabet::Standard,
                                )
                                .unwrap()
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::decode(
                                    black_box(&input),
                                    black_box(&mut main_out),
                                    crate::Alphabet::Standard,
                                )
                                .unwrap()
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 4];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][1]).collect();
        let observed = {
            diffs.sort_by(f64::total_cmp);
            diffs[rounds / 2]
        };
        // Deterministic bootstrap of the paired difference median.
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut boots = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            let mut resample = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                resample.push(diffs[(state as usize) % rounds]);
            }
            resample.sort_by(f64::total_cmp);
            boots.push(resample[rounds / 2]);
        }
        boots.sort_by(f64::total_cmp);
        let lo = boots[250];
        let hi = boots[9_750];
        let pct = observed / med[1] * 100.0;
        println!(
            "{size},{:.3},{:.3},{:.3},{:.3},{observed:.3} [{lo:.3},{hi:.3}],{pct:+.1}%",
            med[0], med[1], med[2], med[3]
        );
    }
}

/// Exploratory encode-variant comparison.
#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_variant_lab() {
    use std::mem::MaybeUninit;
    let sizes = [
        32usize, 48, 64, 96, 128, 256, 512, 1024, 4096, 8192, 65536, 1048576,
    ];
    let rounds = 21usize;
    println!(
        "size,public_ns,direct_ns,load16_ns,unroll8_ns,load16_vs_direct_ns,unroll8_vs_direct_ns"
    );
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut out = vec![0u8; encoded];
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut load16_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut unroll8_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 50_000);

        let mut samples = [[0f64; 4]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..4 {
                let which = (step + round) % 4;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(
                                crate::STANDARD
                                    .encode(black_box(&input), black_box(&mut out))
                                    .unwrap(),
                            );
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        2 => {
                            black_box(unsafe {
                                crate::avx2::encode_load16(
                                    black_box(&input),
                                    black_box(&mut load16_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8(
                                    black_box(&input),
                                    black_box(&mut unroll8_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 4];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][1]).collect();
        diffs.sort_by(f64::total_cmp);
        let observed = diffs[rounds / 2];
        let mut diffs8: Vec<f64> = (0..rounds).map(|r| samples[r][3] - samples[r][1]).collect();
        diffs8.sort_by(f64::total_cmp);
        let observed8 = diffs8[rounds / 2];
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut boots = Vec::with_capacity(10_000);
        for _ in 0..10_000 {
            let mut resample = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                resample.push(diffs[(state as usize) % rounds]);
            }
            resample.sort_by(f64::total_cmp);
            boots.push(resample[rounds / 2]);
        }
        boots.sort_by(f64::total_cmp);
        let lo = boots[250];
        let hi = boots[9_750];
        let pct = observed / med[1] * 100.0;
        let pct8 = observed8 / med[1] * 100.0;
        println!(
            "{size},{:.3},{:.3},{:.3},{:.3},{observed:.3} [{lo:.3},{hi:.3}],{observed8:.3},{pct:+.1}%,{pct8:+.1}%",
            med[0], med[1], med[2], med[3]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_countloop_lab() {
    use std::mem::MaybeUninit;
    let sizes = [
        192usize, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 8192, 65536, 262144, 1048576,
    ];
    let rounds = 21usize;
    println!("size,direct_ns,countloop_ns,diff_ns");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut count_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 2]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..2 {
                let which = (step + round) % 2;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    if which == 0 {
                        black_box(unsafe {
                            crate::avx2::encode(
                                black_box(&input),
                                black_box(&mut direct_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    } else {
                        black_box(unsafe {
                            crate::avx2::lab::encode_countloop(
                                black_box(&input),
                                black_box(&mut count_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 2];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
        diffs.sort_by(f64::total_cmp);
        println!(
            "{size},{:.3},{:.3},{:.3}",
            med[0],
            med[1],
            diffs[rounds / 2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_hybrid_lab() {
    use std::mem::MaybeUninit;
    let sizes = [
        192usize, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 8192, 65536, 262144, 1048576,
    ];
    let rounds = 21usize;
    println!("size,direct_ns,hybrid_ns,diff_ns");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut hybrid_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 2]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..2 {
                let which = (step + round) % 2;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    if which == 0 {
                        black_box(unsafe {
                            crate::avx2::encode(
                                black_box(&input),
                                black_box(&mut direct_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    } else {
                        black_box(unsafe {
                            crate::avx2::lab::encode_hybrid(
                                black_box(&input),
                                black_box(&mut hybrid_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 2];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
        diffs.sort_by(f64::total_cmp);
        println!(
            "{size},{:.3},{:.3},{:.3}",
            med[0],
            med[1],
            diffs[rounds / 2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_unroll8seq_lab() {
    use std::mem::MaybeUninit;
    let sizes = [
        192usize, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 8192, 65536, 262144, 1048576,
    ];
    let rounds = 21usize;
    println!("size,direct_ns,unroll8seq_ns,diff_ns");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 2]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..2 {
                let which = (step + round) % 2;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    if which == 0 {
                        black_box(unsafe {
                            crate::avx2::encode(
                                black_box(&input),
                                black_box(&mut direct_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    } else {
                        black_box(unsafe {
                            crate::avx2::lab::encode_unroll8_seq(
                                black_box(&input),
                                black_box(&mut seq_out),
                                crate::Alphabet::Standard,
                            )
                        });
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 2];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
        diffs.sort_by(f64::total_cmp);
        println!(
            "{size},{:.3},{:.3},{:.3}",
            med[0],
            med[1],
            diffs[rounds / 2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_public_lab() {
    use std::sync::atomic::Ordering;
    let sizes = [
        192usize, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096, 8192, 16384, 65536, 262144,
        1048576,
    ];
    let rounds = 21usize;
    // 0 = current auto, 1 = direct, 2 = unroll8_seq, 3 = countloop.
    let variants = [0u8, 1, 2, 3];
    println!("size,auto_ns,direct_ns,seq_ns,count_ns,seq_diff,count_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut outs: Vec<Vec<u8>> = (0..variants.len()).map(|_| vec![0u8; encoded]).collect();
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 4]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..variants.len() {
                let which = (step + round) % variants.len();
                let variant = variants[which];
                crate::avx2::lab::ENCODE_VARIANT.store(variant, Ordering::Relaxed);
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    black_box(
                        crate::STANDARD
                            .encode(black_box(&input), black_box(&mut outs[which]))
                            .unwrap(),
                    );
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
        let mut med = [0f64; 4];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][1]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let count_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][3] - samples[r][1]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{:.3},{seq_diff:.3},{count_diff:.3}",
            med[0], med[1], med[2], med[3]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_mixed_lab() {
    use std::mem::MaybeUninit;
    let sizes = [8192usize, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,mixed_ns,seq_diff,mixed_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut mixed_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_mixed(
                                    black_box(&input),
                                    black_box(&mut mixed_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let mixed_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{mixed_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_phased_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,phased_ns,seq_diff,phased_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut phased_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_load16_phased(
                                    black_box(&input),
                                    black_box(&mut phased_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let phased_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{phased_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_2phase_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,twophase_ns,seq_diff,twophase_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut two_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_2phase(
                                    black_box(&input),
                                    black_box(&mut two_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let two_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{two_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_split_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,split_ns,seq_diff,split_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut split_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::encode_unroll8_split::<0>(
                                    black_box(&input),
                                    black_box(&mut split_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let split_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{split_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_load16u8_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,l16u8_ns,seq_diff,l16u8_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut l16_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_load16_u8(
                                    black_box(&input),
                                    black_box(&mut l16_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let l16_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{l16_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(all(target_arch = "x86_64", target_os = "linux"))]
fn encode_asm_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,asm_ns,seq_diff,asm_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut asm_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_unroll8_seq(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_asm_load16(
                                    black_box(&input),
                                    black_box(&mut asm_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let asm_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{asm_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_public_split_lab() {
    use std::sync::atomic::Ordering;
    let sizes = [2048usize, 3072, 4096, 8192, 16384, 65536];
    let rounds = 21usize;
    println!("size,direct_ns,split_ns,diff_ns");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![0u8; encoded];
        let mut split_out = vec![0u8; encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 2]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..2 {
                let which = (step + round) % 2;
                crate::avx2::lab::ENCODE_VARIANT.store(if which == 0 { 1 } else { 5 }, Ordering::Relaxed);
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    black_box(
                        crate::STANDARD
                            .encode(
                                black_box(&input),
                                if which == 0 {
                                    black_box(&mut direct_out)
                                } else {
                                    black_box(&mut split_out)
                                },
                            )
                            .unwrap(),
                    );
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
        let mut med = [0f64; 2];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let mut diffs: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
        diffs.sort_by(f64::total_cmp);
        println!(
            "{size},{:.3},{:.3},{:.3}",
            med[0],
            med[1],
            diffs[rounds / 2]
        );
    }
}

#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_turbo8_lab() {
    use std::mem::MaybeUninit;
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576];
    let rounds = 21usize;
    println!("size,direct_ns,seq_ns,turbo8_ns,seq_diff,turbo8_diff");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = crate::STANDARD.encoded_len(size).unwrap();
        let mut direct_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut seq_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let mut t8_out = vec![MaybeUninit::<u8>::uninit(); encoded];
        let iters = (4 * 1024 * 1024 / size).clamp(8, 10_000);
        let mut samples = [[0f64; 3]; 21];
        for (round, sample) in samples.iter_mut().enumerate() {
            for step in 0..3 {
                let which = (step + round) % 3;
                let start = std::time::Instant::now();
                for _ in 0..iters {
                    match which {
                        0 => {
                            black_box(unsafe {
                                crate::avx2::encode(
                                    black_box(&input),
                                    black_box(&mut direct_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        1 => {
                            black_box(unsafe {
                                crate::avx2::encode_unroll8_split::<0>(
                                    black_box(&input),
                                    black_box(&mut seq_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                        _ => {
                            black_box(unsafe {
                                crate::avx2::lab::encode_load16_turbo8(
                                    black_box(&input),
                                    black_box(&mut t8_out),
                                    crate::Alphabet::Standard,
                                )
                            });
                        }
                    }
                }
                sample[which] = start.elapsed().as_secs_f64() * 1e9 / iters as f64;
            }
        }
        let mut med = [0f64; 3];
        for (i, m) in med.iter_mut().enumerate() {
            let mut values: Vec<f64> = (0..rounds).map(|r| samples[r][i]).collect();
            values.sort_by(f64::total_cmp);
            *m = values[rounds / 2];
        }
        let seq_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][1] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        let t8_diff = {
            let mut d: Vec<f64> = (0..rounds).map(|r| samples[r][2] - samples[r][0]).collect();
            d.sort_by(f64::total_cmp);
            d[rounds / 2]
        };
        println!(
            "{size},{:.3},{:.3},{:.3},{seq_diff:.3},{t8_diff:.3}",
            med[0], med[1], med[2]
        );
    }
}

/// Fine-grained encode crossover sweep. Run with:
/// `cargo test --release encode_crossover_lab -- --ignored --nocapture`
#[test]
#[ignore = "timing, run separately under the benchmark lock"]
#[cfg(target_arch = "x86_64")]
fn encode_crossover_lab() {
    use std::sync::atomic::Ordering;
    let mut sizes = Vec::new();
    sizes.extend(1..=64usize);
    for s in (68..=128).step_by(4) {
        sizes.push(s);
    }
    for s in (132..=256).step_by(4) {
        sizes.push(s);
    }
    for s in (264..=512).step_by(8) {
        sizes.push(s);
    }
    for s in (520..=1024).step_by(16) {
        sizes.push(s);
    }
    for s in (1056..=4096).step_by(64) {
        sizes.push(s);
    }
    for s in (4160..=16384).step_by(256) {
        sizes.push(s);
    }
    for s in (16512..=65536).step_by(1024) {
        sizes.push(s);
    }
    sizes.extend([97, 192, 1024, 4096, 4608, 5120, 65536]);
    sizes.sort_unstable();
    sizes.dedup();
    let variants = [
        (0u8, "auto"),
        (1, "direct"),
        (4, "load16"),
        (5, "split"),
        (6, "split_nt32"),
        (7, "split_nt16"),
        (8, "legacy16"),
    ];
    let rounds = 9usize;
    println!("size,variant,ns_per_call,tsc_cycles_per_call,ns_per_byte,tsc_cycles_per_byte");
    for &size in &sizes {
        let mut input_store = vec![0u8; size + 64];
        let input_offset = (64 - input_store.as_ptr() as usize % 64) % 64;
        let input = &mut input_store[input_offset..input_offset + size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let cap = crate::STANDARD.encoded_len(size).unwrap() + 64;
        let mut output_store = vec![0u8; cap + 64];
        let output_offset = (64 - output_store.as_ptr() as usize % 64) % 64;
        let output = &mut output_store[output_offset..output_offset + cap];
        let iters = (256 * 1024 / size.max(1)).clamp(64, 20_000);
        for &(variant, name) in &variants {
            crate::avx2::lab::ENCODE_VARIANT.store(variant, Ordering::Relaxed);
            let mut ns_samples = Vec::with_capacity(rounds);
            let mut cycle_samples = Vec::with_capacity(rounds);
            for _ in 0..rounds {
                let start_t = std::time::Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    black_box(
                        crate::STANDARD
                            .encode(black_box(input), black_box(&mut *output))
                            .unwrap(),
                    );
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                let ns = start_t.elapsed().as_secs_f64() * 1e9 / iters as f64;
                let cycles = (end_c.wrapping_sub(start_c)) as f64 / iters as f64;
                ns_samples.push(ns);
                cycle_samples.push(cycles);
            }
            crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
            ns_samples.sort_by(f64::total_cmp);
            cycle_samples.sort_by(f64::total_cmp);
            let ns = ns_samples[rounds / 2];
            let cycles = cycle_samples[rounds / 2];
            println!(
                "{size},{name},{ns:.4},{cycles:.4},{:.4},{:.4}",
                ns / size as f64,
                cycles / size as f64
            );
        }
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn streaming_encode_matches_reference() {
    use base64::Engine as _;

    if !crate::cpu::avx2_available() {
        return;
    }
    let floor = crate::cpu::stream_floor();
    if floor == usize::MAX || floor > 32 * 1024 * 1024 {
        return;
    }
    let size = floor + 4096 + 7;
    let input: Vec<u8> = (0..size).map(|i| (i * 73 + 19) as u8).collect();
    let url_codec = crate::Codec {
        alphabet: Alphabet::UrlSafe,
        padding: true,
        ignore_whitespace: false,
    };
    let cases = [
        (
            crate::STANDARD,
            base64::engine::general_purpose::STANDARD.encode(&input),
        ),
        (
            url_codec,
            base64::engine::general_purpose::URL_SAFE.encode(&input),
        ),
    ];
    let cap = crate::STANDARD.encoded_len(size).unwrap();
    let mut storage = vec![0u8; cap + 65];
    for (codec, expected) in cases {
        for offset in [0usize, 1] {
            let start = (64 - storage.as_ptr() as usize % 64) % 64 + offset;
            let output = &mut storage[start..start + cap];
            let n = codec.encode(&input, output).unwrap();
            assert_eq!(n, cap);
            assert_eq!(&output[..n], expected.as_bytes());
        }
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn encode_16_matches_reference() {
    use base64::Engine as _;

    if !crate::cpu::avx2_available() {
        return;
    }
    let url = crate::Codec {
        alphabet: Alphabet::UrlSafe,
        padding: true,
        ignore_whitespace: false,
    };
    let unpadded = crate::Codec {
        alphabet: Alphabet::Standard,
        padding: false,
        ignore_whitespace: false,
    };
    for &size in &[8usize, 16, 32] {
        let input: Vec<u8> = (0..size).map(|i| (i * 73 + 19) as u8).collect();
        for (codec, expected) in [
            (
                crate::STANDARD,
                base64::engine::general_purpose::STANDARD.encode(&input),
            ),
            (
                url,
                base64::engine::general_purpose::URL_SAFE.encode(&input),
            ),
            (
                unpadded,
                base64::engine::general_purpose::STANDARD_NO_PAD.encode(&input),
            ),
        ] {
            let cap = codec.encoded_len(input.len()).unwrap();
            let mut output = vec![0u8; cap];
            let n = codec.encode(&input, &mut output).unwrap();
            assert_eq!(n, cap);
            assert_eq!(&output[..n], expected.as_bytes(), "size={size}");
        }
    }
}

#[test]
#[cfg(target_arch = "x86_64")]
fn encode_streaming_api_matches_reference() {
    use base64::Engine as _;

    if !crate::cpu::avx2_available() {
        return;
    }
    let url_codec = crate::Codec {
        alphabet: Alphabet::UrlSafe,
        padding: true,
        ignore_whitespace: false,
    };
    for &size in &[4096usize, 64 * 1024, 1024 * 1024] {
        let input: Vec<u8> = (0..size).map(|i| (i * 73 + 19) as u8).collect();
        let cases = [
            (
                crate::STANDARD,
                base64::engine::general_purpose::STANDARD.encode(&input),
            ),
            (
                url_codec,
                base64::engine::general_purpose::URL_SAFE.encode(&input),
            ),
        ];
        let cap = crate::STANDARD.encoded_len(size).unwrap();
        let mut storage = vec![0u8; cap + 65];
        for (codec, expected) in cases {
            for offset in [0usize, 1] {
                let start = (64 - storage.as_ptr() as usize % 64) % 64 + offset;
                let output = &mut storage[start..start + cap];
                let n = codec.encode_streaming(&input, output).unwrap();
                assert_eq!(n, cap, "size={size} offset={offset}");
                assert_eq!(
                    &output[..n],
                    expected.as_bytes(),
                    "size={size} offset={offset}"
                );
            }
        }
    }
}

/// Streaming-store experiment for buffers that exceed the last-level cache.
/// Run with `cargo test --release --lib encode_nt_lab -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run separately"]
#[cfg(target_arch = "x86_64")]
fn encode_nt_lab() {
    use std::sync::atomic::Ordering;

    let sizes = [
        4_194_304usize,
        4_718_592,
        5_242_880,
        5_767_168,
        6_291_456,
        6_815_744,
        7_340_032,
        7_864_320,
        8_388_608,
        12_582_912,
        16_777_216,
    ];
    let variants = [
        (0u8, "auto", 8192usize, false),
        (1, "direct", 8192, false),
        (5, "split", 8192, false),
        (6, "split_nt32", 8192, false),
        (7, "split_nt16", 8192, false),
        (7, "split_nt16_nta", 8192, true),
        (7, "split_nt16_p16k", 16384, false),
        (7, "split_nt16_p4k", 4096, false),
        (7, "split_nt16_nop", 0, false),
        (7, "split_nt16_p32k", 32768, false),
        (5, "split_p4k", 4096, false),
        (5, "split_p16k", 16384, false),
        (5, "split_nop", 0, false),
    ];
    let rounds = 8usize;
    println!("size,variant,ns_per_call,gbps,tsc_cycles_per_byte");
    for &size in &sizes {
        let mut input_store = vec![0u8; size + 64];
        let input_offset = (64 - input_store.as_ptr() as usize % 64) % 64;
        let input = &mut input_store[input_offset..input_offset + size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let cap = crate::STANDARD.encoded_len(size).unwrap() + 64;
        let mut output_store = vec![0u8; cap + 64];
        let output_offset = (64 - output_store.as_ptr() as usize % 64) % 64;
        let output = &mut output_store[output_offset..output_offset + cap];
        // Keep each sample near 64 MiB of input traffic.
        let iters = (64 * 1024 * 1024 / size).clamp(2, 256);
        let mut series: Vec<Vec<f64>> = variants
            .iter()
            .map(|_| Vec::with_capacity(rounds))
            .collect();
        let mut cycles: Vec<Vec<f64>> = variants
            .iter()
            .map(|_| Vec::with_capacity(rounds))
            .collect();
        for round in 0..rounds {
            for step in 0..variants.len() {
                let idx = (step + round) % variants.len();
                let (variant, _, prefetch_distance, prefetch_nta) = variants[idx];
                crate::avx2::lab::ENCODE_VARIANT.store(variant, Ordering::Relaxed);
                crate::avx2::lab::ENCODE_PREFETCH_DISTANCE.store(prefetch_distance, Ordering::Relaxed);
                crate::avx2::lab::ENCODE_PREFETCH_NTA.store(prefetch_nta, Ordering::Relaxed);
                let start_t = std::time::Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    black_box(
                        crate::STANDARD
                            .encode(black_box(input), black_box(&mut *output))
                            .unwrap(),
                    );
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                series[idx].push(start_t.elapsed().as_secs_f64() * 1e9 / iters as f64);
                cycles[idx].push((end_c.wrapping_sub(start_c)) as f64 / iters as f64);
            }
        }
        crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
        crate::avx2::lab::ENCODE_PREFETCH_DISTANCE.store(8192, Ordering::Relaxed);
        crate::avx2::lab::ENCODE_PREFETCH_NTA.store(false, Ordering::Relaxed);
        for (idx, (_, name, _, _)) in variants.iter().enumerate() {
            series[idx].sort_by(f64::total_cmp);
            cycles[idx].sort_by(f64::total_cmp);
            let ns = series[idx][rounds / 2];
            let cycle = cycles[idx][rounds / 2];
            println!(
                "{size},{name},{ns:.4},{:.4},{:.4}",
                size as f64 / ns,
                cycle / size as f64
            );
        }
    }
}

/// Focused NT prefetch-distance sweep for the 8–16 MiB region.
/// Run with `cargo test --release --lib encode_prefetch_lab -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run separately"]
#[cfg(target_arch = "x86_64")]
fn encode_prefetch_lab() {
    use std::sync::atomic::Ordering;

    let sizes = [
        8 * 1024 * 1024usize,
        10 * 1024 * 1024,
        12 * 1024 * 1024,
        14 * 1024 * 1024,
        16 * 1024 * 1024,
    ];
    let variants = [
        (8192usize, false, false, "p8k"),
        (8192, false, true, "p8k_w"),
        (12288, false, true, "p12k_w"),
        (16384, false, true, "p16k_w"),
        (32768, false, true, "p32k_w"),
        (8192, true, true, "nta8k_w"),
        (0, false, false, "nop"),
    ];
    let rounds = 9usize;
    println!("size,variant,ns_per_call,gbps,tsc_cycles_per_byte");
    for &size in &sizes {
        let mut input_store = vec![0u8; size + 64];
        let input_offset = (64 - input_store.as_ptr() as usize % 64) % 64;
        let input = &mut input_store[input_offset..input_offset + size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let cap = crate::STANDARD.encoded_len(size).unwrap() + 64;
        let mut output_store = vec![0u8; cap + 64];
        let output_offset = (64 - output_store.as_ptr() as usize % 64) % 64;
        let output = &mut output_store[output_offset..output_offset + cap];
        let iters = (64 * 1024 * 1024 / size).clamp(2, 256);
        let mut series: Vec<Vec<f64>> = variants
            .iter()
            .map(|_| Vec::with_capacity(rounds))
            .collect();
        let mut cycles: Vec<Vec<f64>> = variants
            .iter()
            .map(|_| Vec::with_capacity(rounds))
            .collect();
        for round in 0..rounds {
            for step in 0..variants.len() {
                let idx = (step + round) % variants.len();
                let (distance, nta, wide, _) = variants[idx];
                crate::avx2::lab::ENCODE_VARIANT.store(7, Ordering::Relaxed);
                crate::avx2::lab::ENCODE_PREFETCH_DISTANCE.store(distance, Ordering::Relaxed);
                crate::avx2::lab::ENCODE_PREFETCH_NTA.store(nta, Ordering::Relaxed);
                crate::avx2::lab::ENCODE_PREFETCH_WIDE.store(wide, Ordering::Relaxed);
                let start_t = std::time::Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    black_box(
                        crate::STANDARD
                            .encode(black_box(input), black_box(&mut *output))
                            .unwrap(),
                    );
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                series[idx].push(start_t.elapsed().as_secs_f64() * 1e9 / iters as f64);
                cycles[idx].push((end_c.wrapping_sub(start_c)) as f64 / iters as f64);
            }
        }
        crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
        crate::avx2::lab::ENCODE_PREFETCH_DISTANCE.store(8192, Ordering::Relaxed);
        crate::avx2::lab::ENCODE_PREFETCH_NTA.store(false, Ordering::Relaxed);
        crate::avx2::lab::ENCODE_PREFETCH_WIDE.store(false, Ordering::Relaxed);
        for (idx, (_, _, _, name)) in variants.iter().enumerate() {
            series[idx].sort_by(f64::total_cmp);
            cycles[idx].sort_by(f64::total_cmp);
            let ns = series[idx][rounds / 2];
            let cycle = cycles[idx][rounds / 2];
            println!(
                "{size},{name},{ns:.4},{:.4},{:.4}",
                size as f64 / ns,
                cycle / size as f64
            );
        }
    }
}

/// Cold-buffer variant sweep: does streaming help when the input and output
/// are not cache-resident? Run with
/// `cargo test --release --lib encode_cold_lab -- --ignored --nocapture`.
#[test]
#[ignore = "timing, run separately"]
#[cfg(target_arch = "x86_64")]
fn encode_cold_lab() {
    use std::sync::atomic::Ordering;

    fn align_up(value: usize, align: usize) -> usize {
        value.div_ceil(align) * align
    }
    fn gcd(mut a: usize, mut b: usize) -> usize {
        while b != 0 {
            let t = a % b;
            a = b;
            b = t;
        }
        a
    }

    let sizes = [
        256usize,
        1024,
        4 * 1024,
        64 * 1024,
        1024 * 1024,
        4 * 1024 * 1024,
        8 * 1024 * 1024,
        16 * 1024 * 1024,
    ];
    let variants = [(0u8, "auto"), (5, "split"), (7, "nt16")];
    let rounds = 5usize;
    println!("size,variant,gbps,tsc_cycles_per_byte");
    for &size in &sizes {
        let cap = crate::STANDARD.encoded_len(size).unwrap();
        let in_stride = align_up(size, 64);
        let out_stride = align_up(cap, 64);
        let count = ((64 * 1024 * 1024) / (in_stride + out_stride)).max(2);
        let mut input = vec![0u8; count * in_stride + 64];
        let mut output = vec![0u8; count * out_stride + 64];
        let in_start = (64 - input.as_ptr() as usize % 64) % 64;
        let out_start = (64 - output.as_ptr() as usize % 64) % 64;
        for slot in 0..count {
            let base = in_start + slot * in_stride;
            for i in 0..size {
                input[base + i] = ((slot * 131 + i * 73 + 19) & 0xff) as u8;
            }
            let base = out_start + slot * out_stride;
            output[base..base + cap].fill(0);
        }
        let mut step = 7919 % count;
        while step > 1 && gcd(step, count) != 1 {
            step -= 1;
        }
        let step = step.max(1);
        let iters = (32 * 1024 * 1024 / size).clamp(4, 8192);
        for &(variant, name) in &variants {
            crate::avx2::lab::ENCODE_VARIANT.store(variant, Ordering::Relaxed);
            crate::avx2::lab::ENCODE_PREFETCH_DISTANCE.store(8192, Ordering::Relaxed);
            crate::avx2::lab::ENCODE_PREFETCH_NTA.store(false, Ordering::Relaxed);
            crate::avx2::lab::ENCODE_PREFETCH_WIDE.store(true, Ordering::Relaxed);
            let mut samples = Vec::with_capacity(rounds);
            let mut cycles = Vec::with_capacity(rounds);
            for round in 0..rounds {
                let mut slot = round % count;
                let start_t = std::time::Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    let in_base = in_start + slot * in_stride;
                    let out_base = out_start + slot * out_stride;
                    black_box(
                        crate::STANDARD
                            .encode(
                                black_box(&input[in_base..in_base + size]),
                                black_box(&mut output[out_base..out_base + cap]),
                            )
                            .unwrap(),
                    );
                    slot = (slot + step) % count;
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                samples.push(start_t.elapsed().as_secs_f64() * 1e9 / iters as f64);
                cycles.push((end_c.wrapping_sub(start_c)) as f64 / iters as f64 / size as f64);
            }
            crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
            samples.sort_by(f64::total_cmp);
            cycles.sort_by(f64::total_cmp);
            let ns = samples[rounds / 2];
            let cycle = cycles[rounds / 2];
            println!("{size},{name},{:.4},{cycle:.4}", size as f64 / ns);
        }
    }
}

/// Allocation and input-fill baseline for `encode_profile_one`. It performs
/// the same setup and the same empty loop, so subtracting its counters leaves
/// only the encode calls.
#[test]
#[ignore = "profiling baseline; run under cachegrind"]
#[cfg(target_arch = "x86_64")]
fn encode_profile_setup() {
    let size: usize = std::env::var("PROFILE_SIZE")
        .unwrap_or_else(|_| "65536".to_string())
        .parse()
        .unwrap();
    let iters: usize = std::env::var("PROFILE_ITERS")
        .unwrap_or_else(|_| "64".to_string())
        .parse()
        .unwrap();
    let mut input = vec![0u8; size];
    for (i, b) in input.iter_mut().enumerate() {
        *b = (i * 73 + 19) as u8;
    }
    let cap = crate::STANDARD.encoded_len(size).unwrap() + 64;
    let mut output = vec![0u8; cap];
    let mut checksum = 0usize;
    for _ in 0..iters {
        checksum = checksum.wrapping_add(black_box(size));
    }
    black_box(checksum);
    black_box(&mut input);
    black_box(&mut output);
}

/// One encode configuration for cachegrind. Set PROFILE_SIZE, PROFILE_VARIANT,
/// and PROFILE_ITERS, then run the test binary under cachegrind.
#[test]
#[ignore = "profiling harness; run under cachegrind"]
#[cfg(target_arch = "x86_64")]
fn encode_profile_one() {
    use std::sync::atomic::Ordering;
    let size: usize = std::env::var("PROFILE_SIZE")
        .unwrap_or_else(|_| "65536".to_string())
        .parse()
        .unwrap();
    let variant: u8 = std::env::var("PROFILE_VARIANT")
        .unwrap_or_else(|_| "0".to_string())
        .parse()
        .unwrap();
    let iters: usize = std::env::var("PROFILE_ITERS")
        .unwrap_or_else(|_| "64".to_string())
        .parse()
        .unwrap();
    let mut input = vec![0u8; size];
    for (i, b) in input.iter_mut().enumerate() {
        *b = (i * 73 + 19) as u8;
    }
    let cap = crate::STANDARD.encoded_len(size).unwrap() + 64;
    let mut output = vec![0u8; cap];
    crate::avx2::lab::ENCODE_VARIANT.store(variant, Ordering::Relaxed);
    let mut checksum = 0usize;
    for _ in 0..iters {
        checksum = checksum.wrapping_add(
            crate::STANDARD
                .encode(black_box(&input), black_box(&mut output))
                .unwrap(),
        );
    }
    crate::avx2::lab::ENCODE_VARIANT.store(0, Ordering::Relaxed);
    black_box(checksum);
}

#[test]
#[ignore = "profiling baseline; run under cachegrind"]
fn encode_profile_empty() {
    let size: usize = std::env::var("PROFILE_SIZE")
        .unwrap_or_else(|_| "65536".to_string())
        .parse()
        .unwrap();
    let iters: usize = std::env::var("PROFILE_ITERS")
        .unwrap_or_else(|_| "64".to_string())
        .parse()
        .unwrap();
    let mut checksum = 0usize;
    for _ in 0..iters {
        checksum = checksum.wrapping_add(black_box(size));
    }
    black_box(checksum);
}
