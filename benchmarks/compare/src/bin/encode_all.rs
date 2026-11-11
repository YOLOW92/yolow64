use std::ffi::c_char;
use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

use base64::Engine as _;
use base64_simd::AsOut as _;

const ROUNDS: usize = 7;
const SHORT: &[usize] = &[8, 12, 16, 24, 32, 48, 64, 96, 128];
const MEDIUM: &[usize] = &[192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096];
const LARGE: &[usize] = &[8192, 16384, 65536, 262144, 1048576, 4194304, 16777216];
const URL_SIZES: &[usize] = &[64, 4096, 65536, 1048576];
const ALIGN_SIZES: &[usize] = &[64, 4096, 65536];
const ALIGNMENTS: &[usize] = &[0, 1, 8, 16, 32];

fn encoded_len(n: usize) -> usize {
    n.div_ceil(3) * 4
}

fn url_encoded_len(n: usize) -> usize {
    n / 3 * 4
        + match n % 3 {
            0 => 0,
            1 => 2,
            _ => 3,
        }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Lib {
    Yolow64,
    Base64Scalar,
    Base64Simd,
    Base64SimdCrate,
    Base64Turbo,
    Encodify,
    Simdutf,
    TurboBase64,
    Aklomp,
    FastScalar,
    FastChromium,
    FastKlomp,
    FastAvx2,
    Base64Ng,
    DataEncoding,
    Base64ct,
    Base64Light,
    SimpleBase64,
    TapeBase64,
    Radix64Std,
    Fast32,
    Base64UrlCrate,
}

impl Lib {
    fn name(self) -> &'static str {
        match self {
            Self::Yolow64 => "yolow64",
            Self::Base64Scalar => "base64-scalar",
            Self::Base64Simd => "base64-simd-engine",
            Self::Base64SimdCrate => "base64-simd",
            Self::Base64Turbo => "base64-turbo",
            Self::Encodify => "encodify",
            Self::Simdutf => "simdutf",
            Self::TurboBase64 => "Turbo-Base64",
            Self::Aklomp => "aklomp-base64",
            Self::FastScalar => "fastbase64-scalar",
            Self::FastChromium => "fastbase64-chromium",
            Self::FastKlomp => "fastbase64-klomp-avx2",
            Self::FastAvx2 => "fastbase64-fast-avx2",
            Self::Base64Ng => "base64-ng",
            Self::DataEncoding => "data-encoding",
            Self::Base64ct => "base64ct",
            Self::Base64Light => "base64_light",
            Self::SimpleBase64 => "simple-base64",
            Self::TapeBase64 => "tape-base64",
            Self::Radix64Std => "radix64-std",
            Self::Fast32 => "fast32-rfc4648",
            Self::Base64UrlCrate => "base64-url",
        }
    }

    fn allocation(self) -> bool {
        matches!(
            self,
            Self::Base64Light | Self::Fast32 | Self::Base64UrlCrate
        )
    }
}

fn libs() -> Vec<Lib> {
    let mut libs = vec![
        Lib::Yolow64,
        Lib::Base64Scalar,
        Lib::Base64Simd,
        Lib::Base64SimdCrate,
        Lib::Base64Turbo,
        Lib::Encodify,
        Lib::Simdutf,
        Lib::Base64Ng,
        Lib::DataEncoding,
        Lib::Base64ct,
        Lib::Base64Light,
        Lib::SimpleBase64,
        Lib::TapeBase64,
        Lib::Radix64Std,
        Lib::Fast32,
    ];
    if cfg!(turbo_base64) {
        libs.push(Lib::TurboBase64);
    }
    if cfg!(aklomp_base64) {
        libs.push(Lib::Aklomp);
    }
    if cfg!(fastbase64) {
        libs.extend([
            Lib::FastScalar,
            Lib::FastChromium,
            Lib::FastKlomp,
            Lib::FastAvx2,
        ]);
    }
    libs
}

struct Aligned {
    mem: Vec<u8>,
    start: usize,
    len: usize,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let mem = vec![0u8; len + 64];
        let start = (64 - mem.as_ptr() as usize % 64) % 64;
        Self { mem, start, len }
    }
    fn slice(&self) -> &[u8] {
        &self.mem[self.start..self.start + self.len]
    }
    fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.mem[self.start..self.start + self.len]
    }
}

fn encode_into(lib: Lib, input: &[u8], output: &mut [u8]) -> usize {
    match lib {
        Lib::Yolow64 => yolow64::STANDARD
            .encode(input, &mut output[..encoded_len(input.len())])
            .unwrap(),
        Lib::Base64Scalar => base64::engine::general_purpose::STANDARD
            .encode_slice(input, &mut output[..encoded_len(input.len())])
            .unwrap(),
        Lib::Base64Simd => base64::engine::Simd::standard(
            base64::engine::general_purpose::GeneralPurposeConfig::new(),
        )
        .encode_slice(input, &mut output[..encoded_len(input.len())])
        .unwrap(),
        Lib::Base64SimdCrate => base64_simd::STANDARD.encode(input, output.as_out()).len(),
        Lib::Base64Turbo => base64_turbo::STANDARD
            .encode_slice(input, &mut output[..encoded_len(input.len())])
            .unwrap(),
        Lib::Encodify => encodify::base64::STANDARD
            .encode_slice(input, &mut output[..encoded_len(input.len())])
            .unwrap(),
        Lib::Simdutf => unsafe {
            simdutf::binary_to_base64(
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                simdutf::Base64Options::Default,
            )
        },
        Lib::TurboBase64 => {
            #[cfg(turbo_base64)]
            unsafe {
                tb64enc(input.as_ptr(), input.len(), output.as_mut_ptr())
            }
            #[cfg(not(turbo_base64))]
            {
                unreachable!()
            }
        }
        Lib::Aklomp => {
            #[cfg(aklomp_base64)]
            unsafe {
                let mut outlen = 0usize;
                base64_encode(
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    &mut outlen,
                    0,
                );
                outlen
            }
            #[cfg(not(aklomp_base64))]
            {
                unreachable!()
            }
        }
        Lib::FastScalar => {
            #[cfg(fastbase64)]
            unsafe {
                let mut outlen = 0usize;
                scalar_base64_encode(
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    &mut outlen,
                );
                outlen
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastChromium => {
            #[cfg(fastbase64)]
            unsafe {
                chromium_base64_encode(
                    output.as_mut_ptr().cast(),
                    input.as_ptr().cast(),
                    input.len(),
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastKlomp => {
            #[cfg(fastbase64)]
            unsafe {
                let mut outlen = 0usize;
                klomp_avx2_base64_encode(
                    input.as_ptr().cast(),
                    input.len(),
                    output.as_mut_ptr().cast(),
                    &mut outlen,
                );
                outlen
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastAvx2 => {
            #[cfg(fastbase64)]
            unsafe {
                fast_avx2_base64_encode(
                    output.as_mut_ptr().cast(),
                    input.as_ptr().cast(),
                    input.len(),
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::Base64Ng => base64_ng::STANDARD
            .encode_slice(input, &mut output[..encoded_len(input.len())])
            .unwrap(),
        Lib::DataEncoding => {
            data_encoding::BASE64.encode_mut(input, &mut output[..encoded_len(input.len())]);
            encoded_len(input.len())
        }
        Lib::Base64ct => {
            use base64ct::Encoding;
            base64ct::Base64::encode(input, &mut output[..encoded_len(input.len())])
                .unwrap()
                .len()
        }
        Lib::Base64Light => {
            let s = base64_light::base64_encode_bytes(input);
            output[..s.len()].copy_from_slice(s.as_bytes());
            s.len()
        }
        Lib::SimpleBase64 => simple_base64::encode_engine_slice(
            input,
            &mut output[..encoded_len(input.len())],
            &simple_base64::engine::general_purpose::STANDARD,
        )
        .unwrap(),
        Lib::TapeBase64 => {
            tape_base64::encode(input, &mut output[..encoded_len(input.len())]).unwrap()
        }
        Lib::Radix64Std => {
            radix64::STD.encode_slice(input, &mut output[..encoded_len(input.len())])
        }
        Lib::Fast32 => {
            let s = fast32::base64::RFC4648.encode(input);
            output[..s.len()].copy_from_slice(s.as_bytes());
            s.len()
        }
        Lib::Base64UrlCrate => {
            let s = base64_url::encode_to_slice(input, output).unwrap();
            s.len()
        }
    }
}

fn measure_encode(lib: Lib, input: &[u8], output: &mut [u8], iters: usize) -> f64 {
    match lib {
        Lib::Yolow64 => measure(
            || {
                black_box(
                    yolow64::STANDARD
                        .encode(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Base64Scalar => measure(
            || {
                black_box(
                    base64::engine::general_purpose::STANDARD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Base64Simd => {
            let engine = base64::engine::Simd::standard(
                base64::engine::general_purpose::GeneralPurposeConfig::new(),
            );
            measure(
                || {
                    black_box(
                        engine
                            .encode_slice(black_box(input), black_box(&mut *output))
                            .unwrap(),
                    );
                },
                iters,
            )
        }
        Lib::Base64SimdCrate => measure(
            || {
                black_box(
                    base64_simd::STANDARD
                        .encode(black_box(input), black_box(&mut *output).as_out())
                        .len(),
                );
            },
            iters,
        ),
        Lib::Base64Turbo => measure(
            || {
                black_box(
                    base64_turbo::STANDARD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Encodify => measure(
            || {
                black_box(
                    encodify::base64::STANDARD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Simdutf => measure(
            || {
                black_box(unsafe {
                    simdutf::binary_to_base64(
                        black_box(input).as_ptr(),
                        input.len(),
                        black_box(&mut *output).as_mut_ptr(),
                        simdutf::Base64Options::Default,
                    )
                });
            },
            iters,
        ),
        Lib::TurboBase64 => {
            #[cfg(turbo_base64)]
            {
                measure(
                    || {
                        black_box(unsafe {
                            tb64enc(
                                black_box(input).as_ptr(),
                                input.len(),
                                black_box(&mut *output).as_mut_ptr(),
                            )
                        });
                    },
                    iters,
                )
            }
            #[cfg(not(turbo_base64))]
            {
                unreachable!()
            }
        }
        Lib::Aklomp => {
            #[cfg(aklomp_base64)]
            {
                measure(
                    || {
                        let mut outlen = 0usize;
                        unsafe {
                            base64_encode(
                                black_box(input).as_ptr().cast(),
                                input.len(),
                                black_box(&mut *output).as_mut_ptr().cast(),
                                &mut outlen,
                                0,
                            );
                        }
                        black_box(outlen);
                    },
                    iters,
                )
            }
            #[cfg(not(aklomp_base64))]
            {
                unreachable!()
            }
        }
        Lib::FastScalar => {
            #[cfg(fastbase64)]
            {
                measure(
                    || {
                        let mut outlen = 0usize;
                        unsafe {
                            scalar_base64_encode(
                                black_box(input).as_ptr().cast(),
                                input.len(),
                                black_box(&mut *output).as_mut_ptr().cast(),
                                &mut outlen,
                            );
                        }
                        black_box(outlen);
                    },
                    iters,
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastChromium => {
            #[cfg(fastbase64)]
            {
                measure(
                    || {
                        black_box(unsafe {
                            chromium_base64_encode(
                                black_box(&mut *output).as_mut_ptr().cast(),
                                black_box(input).as_ptr().cast(),
                                input.len(),
                            )
                        });
                    },
                    iters,
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastKlomp => {
            #[cfg(fastbase64)]
            {
                measure(
                    || {
                        let mut outlen = 0usize;
                        unsafe {
                            klomp_avx2_base64_encode(
                                black_box(input).as_ptr().cast(),
                                input.len(),
                                black_box(&mut *output).as_mut_ptr().cast(),
                                &mut outlen,
                            );
                        }
                        black_box(outlen);
                    },
                    iters,
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::FastAvx2 => {
            #[cfg(fastbase64)]
            {
                measure(
                    || {
                        black_box(unsafe {
                            fast_avx2_base64_encode(
                                black_box(&mut *output).as_mut_ptr().cast(),
                                black_box(input).as_ptr().cast(),
                                input.len(),
                            )
                        });
                    },
                    iters,
                )
            }
            #[cfg(not(fastbase64))]
            {
                unreachable!()
            }
        }
        Lib::Base64Ng => measure(
            || {
                black_box(
                    base64_ng::STANDARD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::DataEncoding => measure(
            || {
                data_encoding::BASE64.encode_mut(
                    black_box(input),
                    black_box(&mut output[..encoded_len(input.len())]),
                );
                black_box(encoded_len(input.len()));
            },
            iters,
        ),
        Lib::Base64ct => measure(
            || {
                use base64ct::Encoding;
                black_box(
                    base64ct::Base64::encode(black_box(input), black_box(&mut *output))
                        .unwrap()
                        .len(),
                );
            },
            iters,
        ),
        Lib::Base64Light => measure(
            || {
                black_box(base64_light::base64_encode_bytes(black_box(input)).len());
            },
            iters,
        ),
        Lib::SimpleBase64 => measure(
            || {
                black_box(
                    simple_base64::encode_engine_slice(
                        black_box(input),
                        black_box(&mut *output),
                        &simple_base64::engine::general_purpose::STANDARD,
                    )
                    .unwrap(),
                );
            },
            iters,
        ),
        Lib::TapeBase64 => measure(
            || {
                black_box(tape_base64::encode(black_box(input), black_box(&mut *output)).unwrap());
            },
            iters,
        ),
        Lib::Radix64Std => measure(
            || {
                black_box(radix64::STD.encode_slice(black_box(&input), black_box(&mut *output)));
            },
            iters,
        ),
        Lib::Fast32 => measure(
            || {
                black_box(fast32::base64::RFC4648.encode(black_box(input)).len());
            },
            iters,
        ),
        Lib::Base64UrlCrate => measure(
            || {
                black_box(
                    base64_url::encode_to_slice(black_box(input), black_box(&mut *output))
                        .unwrap()
                        .len(),
                );
            },
            iters,
        ),
    }
}

#[inline(always)]
fn measure<F: FnMut()>(mut f: F, iters: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_secs_f64() * 1e9 / iters as f64
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn iters_for(size: usize, short: bool) -> usize {
    if short {
        (1024 * 1024 / size.max(1)).clamp(10_000, 200_000)
    } else {
        (8 * 1024 * 1024 / size.max(1)).clamp(8, 500_000)
    }
}

fn main() {
    let mut filter: Option<String> = None;
    let mut out_prefix: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--lib" => filter = args.next(),
            "--out" => out_prefix = args.next(),
            other => panic!("unknown argument: {other}"),
        }
    }
    let mut libs = libs();
    if let Some(name) = &filter {
        libs.retain(|lib| lib.name() == name);
        if libs.is_empty() {
            panic!("unknown library: {name}");
        }
    }
    let mut raw = String::from("mode,size,align,lib,kind,round,ns,gbps\n");
    let mut summary = String::from("mode,size,align,lib,kind,median_ns,gbps\n");

    for (mode, sizes) in [("short", SHORT), ("medium", MEDIUM), ("large", LARGE)] {
        for &size in sizes {
            let mut input = Aligned::new(size);
            for (i, b) in input.slice_mut().iter_mut().enumerate() {
                *b = (i * 73 + 19) as u8;
            }
            let expected = base64::engine::general_purpose::STANDARD.encode(input.slice());
            let cap = expected.len() + 64;
            let mut out = Aligned::new(cap);
            let iters = iters_for(size, size <= 128);

            for &lib in &libs {
                let n = encode_into(lib, input.slice(), out.slice_mut());
                assert_eq!(
                    &out.slice()[..n],
                    expected.as_bytes(),
                    "{lib:?} size {size}"
                );
            }

            let mut series: Vec<Vec<f64>> =
                libs.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();
            for round in 0..ROUNDS {
                for step in 0..libs.len() {
                    let idx = (step + round) % libs.len();
                    let lib = libs[idx];
                    let ns = measure_encode(lib, input.slice(), out.slice_mut(), iters);
                    series[idx].push(ns);
                    writeln!(
                        raw,
                        "{mode},{size},0,{},{},{round},{ns:.6},{:.6}",
                        lib.name(),
                        if lib.allocation() { "alloc" } else { "slice" },
                        size as f64 / ns
                    )
                    .unwrap();
                }
            }
            for (idx, &lib) in libs.iter().enumerate() {
                let med = median(&mut series[idx]);
                writeln!(
                    summary,
                    "{mode},{size},0,{},{},{med:.6},{:.6}",
                    lib.name(),
                    if lib.allocation() { "alloc" } else { "slice" },
                    size as f64 / med
                )
                .unwrap();
            }
        }
    }

    // Alignment sensitivity for the strongest implementations.
    let mut align_libs = vec![
        Lib::Yolow64,
        Lib::Base64Turbo,
        Lib::Encodify,
        Lib::Simdutf,
        Lib::TurboBase64,
        Lib::Base64SimdCrate,
    ];
    if let Some(name) = &filter {
        align_libs.retain(|lib| lib.name() == name);
    }
    for &size in ALIGN_SIZES {
        let backing = Aligned::new(size + 64);
        let expected = base64::engine::general_purpose::STANDARD.encode(&backing.slice()[..size]);
        let cap = expected.len() + 64;
        let mut out = Aligned::new(cap);
        for &align in ALIGNMENTS {
            let offset = align.min(64);
            let input = &backing.slice()[..size + offset];
            let input = &input[offset..offset + size];
            for &lib in &align_libs {
                if lib == Lib::TurboBase64 && !cfg!(turbo_base64) {
                    continue;
                }
                let n = encode_into(lib, input, out.slice_mut());
                assert_eq!(&out.slice()[..n], expected.as_bytes());
                let iters = iters_for(size, size <= 128);
                let mut samples = Vec::with_capacity(ROUNDS);
                for _ in 0..ROUNDS {
                    samples.push(measure_encode(lib, input, out.slice_mut(), iters));
                }
                let med = median(&mut samples);
                writeln!(
                    raw,
                    "align,{size},{align},{},{},{:.6},{:.6}",
                    lib.name(),
                    if lib.allocation() { "alloc" } else { "slice" },
                    med,
                    size as f64 / med
                )
                .unwrap();
                writeln!(
                    summary,
                    "align,{size},{align},{},{},{med:.6},{:.6}",
                    lib.name(),
                    if lib.allocation() { "alloc" } else { "slice" },
                    size as f64 / med
                )
                .unwrap();
            }
        }
    }

    // URL-safe no-padding secondary comparison.
    let mut url_libs = vec![
        Lib::Yolow64,
        Lib::Base64Scalar,
        Lib::Base64SimdCrate,
        Lib::Base64Turbo,
        Lib::Encodify,
        Lib::Simdutf,
        Lib::Base64Ng,
        Lib::DataEncoding,
        Lib::Base64ct,
        Lib::TapeBase64,
        Lib::Radix64Std,
        Lib::Base64UrlCrate,
    ];
    if let Some(name) = &filter {
        url_libs.retain(|lib| lib.name() == name);
    }
    for &size in URL_SIZES {
        let mut input = Aligned::new(size);
        for (i, b) in input.slice_mut().iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let expected = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(input.slice());
        let cap = expected.len() + 64;
        let mut out = Aligned::new(cap);
        for &lib in &url_libs {
            let n = encode_url(lib, input.slice(), out.slice_mut());
            assert_eq!(&out.slice()[..n], expected.as_bytes(), "{lib:?} URL {size}");
            let iters = iters_for(size, size <= 128);
            let mut samples = Vec::with_capacity(ROUNDS);
            for _ in 0..ROUNDS {
                samples.push(measure_url(lib, input.slice(), out.slice_mut(), iters));
            }
            let med = median(&mut samples);
            writeln!(
                raw,
                "url,{size},0,{},url-no-pad,{:.6},{:.6}",
                lib.name(),
                med,
                size as f64 / med
            )
            .unwrap();
            writeln!(
                summary,
                "url,{size},0,{},url-no-pad,{med:.6},{:.6}",
                lib.name(),
                size as f64 / med
            )
            .unwrap();
        }
    }

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&dir).unwrap();
    let (raw_path, summary_path) = match out_prefix {
        Some(prefix) => {
            let base = dir.join(prefix);
            (
                base.with_extension("raw.csv"),
                base.with_extension("summary.csv"),
            )
        }
        None => (
            dir.join("encode_all_raw.csv"),
            dir.join("encode_all_summary.csv"),
        ),
    };
    std::fs::write(&raw_path, raw).unwrap();
    std::fs::write(&summary_path, summary).unwrap();
    eprintln!("wrote {}", dir.display());
}

fn encode_url(lib: Lib, input: &[u8], output: &mut [u8]) -> usize {
    let n = url_encoded_len(input.len());
    match lib {
        Lib::Yolow64 => yolow64::Codec {
            alphabet: yolow64::Alphabet::UrlSafe,
            padding: false,
            ignore_whitespace: false,
        }
        .encode(input, &mut output[..])
        .unwrap(),
        Lib::Base64Scalar => base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode_slice(input, &mut output[..n])
            .unwrap(),
        Lib::Base64SimdCrate => base64_simd::URL_SAFE_NO_PAD
            .encode(input, output.as_out())
            .len(),
        Lib::Base64Turbo => base64_turbo::URL_SAFE_NO_PAD
            .encode_slice(input, &mut output[..])
            .unwrap(),
        Lib::Encodify => encodify::base64::URL_SAFE_NO_PAD
            .encode_slice(input, &mut output[..])
            .unwrap(),
        Lib::Simdutf => unsafe {
            simdutf::binary_to_base64(
                input.as_ptr(),
                input.len(),
                output.as_mut_ptr(),
                simdutf::Base64Options::Url,
            )
        },
        Lib::Base64Ng => base64_ng::URL_SAFE_NO_PAD
            .encode_slice(input, &mut output[..])
            .unwrap(),
        Lib::DataEncoding => {
            data_encoding::BASE64URL_NOPAD.encode_mut(input, &mut output[..n]);
            n
        }
        Lib::Base64ct => {
            use base64ct::Encoding;
            base64ct::Base64UrlUnpadded::encode(input, &mut output[..n])
                .unwrap()
                .len()
        }
        Lib::TapeBase64 => tape_base64::URL_SAFE_NO_PAD
            .encode(input, &mut output[..])
            .unwrap(),
        Lib::Radix64Std => radix64::URL_SAFE_NO_PAD.encode_slice(input, &mut output[..n]),
        Lib::Base64UrlCrate => base64_url::encode_to_slice(input, output).unwrap().len(),
        _ => unreachable!(),
    }
}

fn measure_url(lib: Lib, input: &[u8], output: &mut [u8], iters: usize) -> f64 {
    let n = url_encoded_len(input.len());
    match lib {
        Lib::Yolow64 => {
            let codec = yolow64::Codec {
                alphabet: yolow64::Alphabet::UrlSafe,
                padding: false,
                ignore_whitespace: false,
            };
            measure(
                || {
                    black_box(
                        codec
                            .encode(black_box(input), black_box(&mut *output))
                            .unwrap(),
                    );
                },
                iters,
            )
        }
        Lib::Base64Scalar => measure(
            || {
                black_box(
                    base64::engine::general_purpose::URL_SAFE_NO_PAD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Base64SimdCrate => measure(
            || {
                black_box(
                    base64_simd::URL_SAFE_NO_PAD
                        .encode(black_box(input), black_box(&mut *output).as_out())
                        .len(),
                );
            },
            iters,
        ),
        Lib::Base64Turbo => measure(
            || {
                black_box(
                    base64_turbo::URL_SAFE_NO_PAD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Encodify => measure(
            || {
                black_box(
                    encodify::base64::URL_SAFE_NO_PAD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Simdutf => measure(
            || {
                black_box(unsafe {
                    simdutf::binary_to_base64(
                        black_box(input).as_ptr(),
                        input.len(),
                        black_box(&mut *output).as_mut_ptr(),
                        simdutf::Base64Options::Url,
                    )
                });
            },
            iters,
        ),
        Lib::Base64Ng => measure(
            || {
                black_box(
                    base64_ng::URL_SAFE_NO_PAD
                        .encode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::DataEncoding => measure(
            || {
                data_encoding::BASE64URL_NOPAD
                    .encode_mut(black_box(input), black_box(&mut output[..n]));
                black_box(n);
            },
            iters,
        ),
        Lib::Base64ct => measure(
            || {
                use base64ct::Encoding;
                black_box(
                    base64ct::Base64UrlUnpadded::encode(
                        black_box(input),
                        black_box(&mut output[..n]),
                    )
                    .unwrap()
                    .len(),
                );
            },
            iters,
        ),
        Lib::TapeBase64 => measure(
            || {
                black_box(
                    tape_base64::URL_SAFE_NO_PAD
                        .encode(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Radix64Std => measure(
            || {
                black_box(
                    radix64::URL_SAFE_NO_PAD
                        .encode_slice(black_box(input), black_box(&mut output[..n])),
                );
            },
            iters,
        ),
        Lib::Base64UrlCrate => measure(
            || {
                black_box(
                    base64_url::encode_to_slice(black_box(input), black_box(&mut *output))
                        .unwrap()
                        .len(),
                );
            },
            iters,
        ),
        _ => unreachable!(),
    }
}

#[cfg(aklomp_base64)]
unsafe extern "C" {
    fn base64_encode(
        src: *const c_char,
        srclen: usize,
        out: *mut c_char,
        outlen: *mut usize,
        flags: i32,
    );
}

#[cfg(fastbase64)]
unsafe extern "C" {
    fn scalar_base64_encode(
        src: *const c_char,
        srclen: usize,
        out: *mut c_char,
        outlen: *mut usize,
    );
    fn chromium_base64_encode(dest: *mut c_char, src: *const c_char, len: usize) -> usize;
    fn klomp_avx2_base64_encode(
        src: *const c_char,
        srclen: usize,
        out: *mut c_char,
        outlen: *mut usize,
    );
    fn fast_avx2_base64_encode(dest: *mut c_char, src: *const c_char, len: usize) -> usize;
}

#[cfg(turbo_base64)]
unsafe extern "C" {
    fn tb64enc(in_: *const u8, inlen: usize, out: *mut u8) -> usize;
}
