use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

use base64::Engine as _;
use base64_simd::AsOut as _;

const ROUNDS: usize = 7;
const SHORT_SIZES: &[usize] = &[8, 12, 16, 24, 32, 48, 64, 96, 128];
const MEDIUM_SIZES: &[usize] = &[192, 256, 384, 512, 768, 1024, 1536, 2048, 3072, 4096];
const LARGE_SIZES: &[usize] = &[8192, 16384, 65536, 262144, 1048576, 4194304];
const CRLF_SIZES: &[usize] = &[256, 4096, 65536, 1048576, 4194304];

const YOLOW64: yolow64::Codec = yolow64::Codec {
    alphabet: yolow64::Alphabet::Standard,
    padding: true,
    ignore_whitespace: false,
};
const YOLOW64_WS: yolow64::Codec = yolow64::Codec {
    ignore_whitespace: true,
    ..YOLOW64
};

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
            Self::TurboBase64 => "turbo-base64",
        }
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
    ];
    if cfg!(turbo_base64) {
        libs.push(Lib::TurboBase64);
    }
    libs
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Group {
    Short,
    Medium,
    Large,
    Crlf,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Self::Short => "short",
            Self::Medium => "medium",
            Self::Large => "large",
            Self::Crlf => "crlf-decode",
        }
    }
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

    fn len(&self) -> usize {
        self.len
    }

    fn slice(&self) -> &[u8] {
        &self.mem[self.start..self.start + self.len]
    }

    fn slice_mut(&mut self) -> &mut [u8] {
        &mut self.mem[self.start..self.start + self.len]
    }
}

fn make_input(size: usize) -> Aligned {
    let mut input = Aligned::new(size);
    for (i, b) in input.slice_mut().iter_mut().enumerate() {
        *b = (i * 73 + 19) as u8;
    }
    input
}

fn wrap_crlf(encoded: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded.len() + encoded.len() / 76 * 2 + 2);
    for chunk in encoded.chunks(76) {
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\r\n");
    }
    out
}

fn filter_ws(input: &[u8], output: &mut [u8]) -> usize {
    let mut n = 0;
    for &b in input {
        if !matches!(b, b' ' | b'\t' | b'\r' | b'\n') {
            output[n] = b;
            n += 1;
        }
    }
    n
}

fn encode(lib: Lib, input: &[u8], output: &mut [u8]) -> usize {
    match lib {
        Lib::Yolow64 => YOLOW64.encode(input, output).unwrap(),
        Lib::Base64Scalar => base64::engine::general_purpose::STANDARD
            .encode_slice(input, output)
            .unwrap(),
        Lib::Base64Simd => base64::engine::Simd::standard(
            base64::engine::general_purpose::GeneralPurposeConfig::new(),
        )
        .encode_slice(input, output)
        .unwrap(),
        Lib::Base64SimdCrate => base64_simd::STANDARD.encode(input, output.as_out()).len(),
        Lib::Base64Turbo => base64_turbo::STANDARD.encode_slice(input, output).unwrap(),
        Lib::Encodify => encodify::base64::STANDARD
            .encode_slice(input, output)
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
    }
}

fn decode(lib: Lib, input: &[u8], output: &mut [u8]) -> usize {
    match lib {
        Lib::Yolow64 => YOLOW64.decode(input, output).unwrap(),
        Lib::Base64Scalar => base64::engine::general_purpose::STANDARD
            .decode_slice(input, output)
            .unwrap(),
        Lib::Base64Simd => base64::engine::Simd::standard(
            base64::engine::general_purpose::GeneralPurposeConfig::new(),
        )
        .decode_slice(input, output)
        .unwrap(),
        Lib::Base64SimdCrate => base64_simd::STANDARD
            .decode(input, output.as_out())
            .unwrap()
            .len(),
        Lib::Base64Turbo => base64_turbo::STANDARD.decode_slice(input, output).unwrap(),
        Lib::Encodify => encodify::base64::STANDARD
            .decode_slice(input, output)
            .unwrap(),
        Lib::Simdutf => {
            let result = unsafe {
                simdutf::base64_to_binary(
                    input.as_ptr(),
                    input.len(),
                    output.as_mut_ptr(),
                    simdutf::Base64Options::Default,
                    simdutf::LastChunkHandlingOptions::Loose,
                )
            };
            assert_eq!(result.error, simdutf::ErrorCode::Success);
            result.count
        }
        Lib::TurboBase64 => {
            #[cfg(turbo_base64)]
            unsafe {
                let n = tb64dec(input.as_ptr(), input.len(), output.as_mut_ptr());
                assert!(n > 0);
                n
            }
            #[cfg(not(turbo_base64))]
            {
                unreachable!()
            }
        }
    }
}

fn decode_crlf(
    lib: Lib,
    wire: &[u8],
    filtered: &mut [u8],
    output: &mut [u8],
) -> (usize, &'static str) {
    match lib {
        Lib::Yolow64 => (YOLOW64_WS.decode(wire, output).unwrap(), "native"),
        Lib::Encodify => (
            encodify::base64::MIME.decode_slice(wire, output).unwrap(),
            "native",
        ),
        Lib::Base64SimdCrate => (
            base64_simd::forgiving_decode(wire, output.as_out())
                .unwrap()
                .len(),
            "native",
        ),
        Lib::Simdutf => {
            let result = unsafe {
                simdutf::base64_to_binary(
                    wire.as_ptr(),
                    wire.len(),
                    output.as_mut_ptr(),
                    simdutf::Base64Options::Default,
                    simdutf::LastChunkHandlingOptions::Loose,
                )
            };
            assert_eq!(result.error, simdutf::ErrorCode::Success);
            (result.count, "native")
        }
        Lib::Base64Scalar | Lib::Base64Simd | Lib::Base64Turbo | Lib::TurboBase64 => {
            let n = filter_ws(wire, filtered);
            (decode(lib, &filtered[..n], output), "filter+strict")
        }
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

fn measure_encode(lib: Lib, input: &[u8], output: &mut [u8], iters: usize) -> f64 {
    match lib {
        Lib::Yolow64 => measure(
            || {
                black_box(
                    YOLOW64
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
    }
}

fn measure_decode(lib: Lib, input: &[u8], output: &mut [u8], iters: usize) -> f64 {
    match lib {
        Lib::Yolow64 => measure(
            || {
                black_box(
                    YOLOW64
                        .decode(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Base64Scalar => measure(
            || {
                black_box(
                    base64::engine::general_purpose::STANDARD
                        .decode_slice(black_box(input), black_box(&mut *output))
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
                            .decode_slice(black_box(input), black_box(&mut *output))
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
                        .decode(black_box(input), black_box(&mut *output).as_out())
                        .unwrap()
                        .len(),
                );
            },
            iters,
        ),
        Lib::Base64Turbo => measure(
            || {
                black_box(
                    base64_turbo::STANDARD
                        .decode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Encodify => measure(
            || {
                black_box(
                    encodify::base64::STANDARD
                        .decode_slice(black_box(input), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Simdutf => measure(
            || {
                let result = unsafe {
                    simdutf::base64_to_binary(
                        black_box(input).as_ptr(),
                        input.len(),
                        black_box(&mut *output).as_mut_ptr(),
                        simdutf::Base64Options::Default,
                        simdutf::LastChunkHandlingOptions::Loose,
                    )
                };
                debug_assert_eq!(result.error, simdutf::ErrorCode::Success);
                black_box(result.count);
            },
            iters,
        ),
        Lib::TurboBase64 => {
            #[cfg(turbo_base64)]
            {
                measure(
                    || {
                        let n = unsafe {
                            tb64dec(
                                black_box(input).as_ptr(),
                                input.len(),
                                black_box(&mut *output).as_mut_ptr(),
                            )
                        };
                        debug_assert!(n > 0);
                        black_box(n);
                    },
                    iters,
                )
            }
            #[cfg(not(turbo_base64))]
            {
                unreachable!()
            }
        }
    }
}

fn measure_crlf(
    lib: Lib,
    wire: &[u8],
    filtered: &mut [u8],
    output: &mut [u8],
    iters: usize,
) -> f64 {
    match lib {
        Lib::Yolow64 => measure(
            || {
                black_box(
                    YOLOW64_WS
                        .decode(black_box(wire), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Encodify => measure(
            || {
                black_box(
                    encodify::base64::MIME
                        .decode_slice(black_box(wire), black_box(&mut *output))
                        .unwrap(),
                );
            },
            iters,
        ),
        Lib::Base64SimdCrate => measure(
            || {
                black_box(
                    base64_simd::forgiving_decode(
                        black_box(wire),
                        black_box(&mut *output).as_out(),
                    )
                    .unwrap()
                    .len(),
                );
            },
            iters,
        ),
        Lib::Simdutf => measure(
            || {
                let result = unsafe {
                    simdutf::base64_to_binary(
                        black_box(wire).as_ptr(),
                        wire.len(),
                        black_box(&mut *output).as_mut_ptr(),
                        simdutf::Base64Options::Default,
                        simdutf::LastChunkHandlingOptions::Loose,
                    )
                };
                debug_assert_eq!(result.error, simdutf::ErrorCode::Success);
                black_box(result.count);
            },
            iters,
        ),
        Lib::Base64Scalar | Lib::Base64Simd | Lib::Base64Turbo | Lib::TurboBase64 => measure(
            || {
                let n = filter_ws(black_box(wire), &mut filtered[..wire.len()]);
                black_box(decode(lib, &filtered[..n], output));
            },
            iters,
        ),
    }
}

fn iters_for(group: Group, encoded_len: usize, raw_len: usize) -> usize {
    let n = match group {
        Group::Short | Group::Medium => raw_len,
        Group::Large => raw_len,
        Group::Crlf => encoded_len,
    };
    match group {
        Group::Short | Group::Medium => (1_000_000 / n.max(1)).clamp(10_000, 200_000),
        Group::Large | Group::Crlf => (8 * 1024 * 1024 / n.max(1)).clamp(8, 200_000),
    }
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

fn main() {
    let libs = libs();
    let mut raw = String::new();
    writeln!(raw, "group,size,op,lib,kind,round,ns_per_call,gbps").unwrap();

    let mut summary = String::new();
    writeln!(
        summary,
        "group,size,op,lib,kind,median_ns,min_ns,max_ns,gbps"
    )
    .unwrap();

    for (group, sizes) in [
        (Group::Short, SHORT_SIZES),
        (Group::Medium, MEDIUM_SIZES),
        (Group::Large, LARGE_SIZES),
        (Group::Crlf, CRLF_SIZES),
    ] {
        for &size in sizes {
            let input = make_input(size);
            let expected = base64::engine::general_purpose::STANDARD
                .encode(input.slice())
                .into_bytes();
            let mut encoded = Aligned::new(expected.len());
            encoded.slice_mut().copy_from_slice(&expected);

            let wire_bytes = wrap_crlf(&expected);
            let mut wire = Aligned::new(wire_bytes.len());
            wire.slice_mut().copy_from_slice(&wire_bytes);
            let mut filtered = Aligned::new(wire.len());
            let cap = expected.len().max(wire.len()).max(size) + 64;
            let mut out = Aligned::new(cap);

            let encode_iters = iters_for(group, expected.len(), size);
            let decode_iters = iters_for(group, expected.len(), size);

            // Correctness gate before timing.
            let mut encode_ok = Vec::new();
            let mut decode_ok = Vec::new();
            let mut crlf_ok = Vec::new();
            for &lib in &libs {
                let n = encode(lib, input.slice(), out.slice_mut());
                assert_eq!(
                    &out.slice()[..n],
                    expected.as_slice(),
                    "encode mismatch: {lib:?} size {size}"
                );
                encode_ok.push(lib);

                let n = decode(lib, encoded.slice(), out.slice_mut());
                assert_eq!(
                    &out.slice()[..n],
                    input.slice(),
                    "decode mismatch: {lib:?} size {size}"
                );
                decode_ok.push(lib);

                let (n, _) = decode_crlf(lib, wire.slice(), filtered.slice_mut(), out.slice_mut());
                if n == size && &out.slice()[..n] == input.slice() {
                    crlf_ok.push(lib);
                } else {
                    eprintln!("CRLF unsupported or mismatch: {lib:?} size {size}");
                }
            }

            if group != Group::Crlf {
                let mut enc_series: Vec<Vec<f64>> =
                    libs.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();
                let mut dec_series: Vec<Vec<f64>> =
                    libs.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();
                for round in 0..ROUNDS {
                    for step in 0..libs.len() {
                        let idx = (step + round) % libs.len();
                        let lib = libs[idx];
                        if encode_ok.contains(&lib) {
                            let ns =
                                measure_encode(lib, input.slice(), out.slice_mut(), encode_iters);
                            enc_series[idx].push(ns);
                            writeln!(
                                raw,
                                "{},{size},encode,{},strict,{round},{ns:.6},{:.6}",
                                group.name(),
                                lib.name(),
                                size as f64 / ns,
                            )
                            .unwrap();
                        }
                        if decode_ok.contains(&lib) {
                            let ns =
                                measure_decode(lib, encoded.slice(), out.slice_mut(), decode_iters);
                            dec_series[idx].push(ns);
                            writeln!(
                                raw,
                                "{},{size},decode,{},strict,{round},{ns:.6},{:.6}",
                                group.name(),
                                lib.name(),
                                size as f64 / ns,
                            )
                            .unwrap();
                        }
                    }
                }
                for (idx, &lib) in libs.iter().enumerate() {
                    for (op, series) in [
                        ("encode", &mut enc_series[idx]),
                        ("decode", &mut dec_series[idx]),
                    ] {
                        if series.is_empty() {
                            continue;
                        }
                        let min = series.iter().copied().fold(f64::INFINITY, f64::min);
                        let max = series.iter().copied().fold(0.0, f64::max);
                        let med = median(series);
                        let bytes = size;
                        writeln!(
                            summary,
                            "{},{size},{op},{},strict,{med:.6},{min:.6},{max:.6},{:.6}",
                            group.name(),
                            lib.name(),
                            bytes as f64 / med,
                        )
                        .unwrap();
                    }
                }
            } else {
                let mut series: Vec<Vec<f64>> =
                    libs.iter().map(|_| Vec::with_capacity(ROUNDS)).collect();
                for round in 0..ROUNDS {
                    for step in 0..libs.len() {
                        let idx = (step + round) % libs.len();
                        let lib = libs[idx];
                        if !crlf_ok.contains(&lib) {
                            continue;
                        }
                        let ns = measure_crlf(
                            lib,
                            wire.slice(),
                            filtered.slice_mut(),
                            out.slice_mut(),
                            decode_iters,
                        );
                        series[idx].push(ns);
                        let (_, kind) =
                            decode_crlf(lib, wire.slice(), filtered.slice_mut(), out.slice_mut());
                        writeln!(
                            raw,
                            "{},{size},decode,{},{},{round},{ns:.6},{:.6}",
                            group.name(),
                            lib.name(),
                            kind,
                            size as f64 / ns,
                        )
                        .unwrap();
                    }
                }
                for (idx, &lib) in libs.iter().enumerate() {
                    let series = &mut series[idx];
                    if series.is_empty() {
                        continue;
                    }
                    let min = series.iter().copied().fold(f64::INFINITY, f64::min);
                    let max = series.iter().copied().fold(0.0, f64::max);
                    let med = median(series);
                    let (_, kind) =
                        decode_crlf(lib, wire.slice(), filtered.slice_mut(), out.slice_mut());
                    writeln!(
                        summary,
                        "{},{size},decode,{},{kind},{med:.6},{min:.6},{max:.6},{:.6}",
                        group.name(),
                        lib.name(),
                        size as f64 / med,
                    )
                    .unwrap();
                }
            }
        }
    }

    let results_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&results_dir).unwrap();
    std::fs::write(results_dir.join("8way_raw.csv"), &raw).unwrap();
    std::fs::write(results_dir.join("8way_summary.csv"), &summary).unwrap();
    println!("{summary}");
    eprintln!("wrote {}", results_dir.display());
}

#[cfg(turbo_base64)]
unsafe extern "C" {
    fn tb64enc(in_: *const u8, inlen: usize, out: *mut u8) -> usize;
    fn tb64dec(in_: *const u8, inlen: usize, out: *mut u8) -> usize;
}
