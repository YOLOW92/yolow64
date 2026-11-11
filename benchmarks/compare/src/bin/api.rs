use std::hint::black_box;
use std::time::Instant;

const CODEC: yolow64::Codec = yolow64::STANDARD;
const ROUNDS: usize = 5;
const SIZES: &[usize] = &[64 * 1024, 1024 * 1024, 4 * 1024 * 1024];

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 73 + 19) as u8).collect()
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn measure<F: FnMut()>(mut f: F, iters: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_secs_f64() * 1e9 / iters as f64
}

fn main() {
    println!("op,size,median_ns,gbps");
    let mut csv = String::from("op,size,median_ns,gbps\n");
    for &size in SIZES {
        let input = data(size);
        let encoded = CODEC.encode_to_string(&input).unwrap();
        let iters = (32 * 1024 * 1024 / size).clamp(2, 128);
        let mut out = vec![0u8; encoded.len().max(size) + 64];
        let mut append = Vec::with_capacity(encoded.len().max(size) + 64);

        let mut ops: Vec<(&str, Vec<f64>)> = vec![
            ("encode_slice", Vec::new()),
            ("encode_to_string", Vec::new()),
            ("append_encode", Vec::new()),
            ("decode_slice", Vec::new()),
            ("decode_to_vec", Vec::new()),
            ("append_decode", Vec::new()),
            ("stream_oneshot", Vec::new()),
            ("stream_4k", Vec::new()),
        ];

        for _ in 0..ROUNDS {
            for (idx, (_, values)) in ops.iter_mut().enumerate() {
                let ns = match idx {
                    0 => measure(
                        || {
                            black_box(
                                CODEC
                                    .encode(black_box(&input), black_box(&mut out))
                                    .unwrap(),
                            );
                        },
                        iters,
                    ),
                    1 => measure(
                        || {
                            black_box(CODEC.encode_to_string(black_box(&input)).unwrap());
                        },
                        iters,
                    ),
                    2 => measure(
                        || {
                            append.clear();
                            CODEC.append_encode(black_box(&input), &mut append).unwrap();
                            black_box(append.len());
                        },
                        iters,
                    ),
                    3 => measure(
                        || {
                            black_box(
                                CODEC
                                    .decode(black_box(encoded.as_bytes()), black_box(&mut out))
                                    .unwrap(),
                            );
                        },
                        iters,
                    ),
                    4 => measure(
                        || {
                            black_box(CODEC.decode_to_vec(black_box(encoded.as_bytes())).unwrap());
                        },
                        iters,
                    ),
                    5 => measure(
                        || {
                            append.clear();
                            CODEC
                                .append_decode(black_box(encoded.as_bytes()), &mut append)
                                .unwrap();
                            black_box(append.len());
                        },
                        iters,
                    ),
                    6 => measure(
                        || {
                            append.clear();
                            let mut decoder = yolow64::Decoder::new(CODEC);
                            decoder
                                .feed(black_box(encoded.as_bytes()), &mut append)
                                .unwrap();
                            decoder.finish(&mut append).unwrap();
                            black_box(append.len());
                        },
                        iters,
                    ),
                    _ => measure(
                        || {
                            append.clear();
                            let mut decoder = yolow64::Decoder::new(CODEC);
                            for chunk in encoded.as_bytes().chunks(4096) {
                                decoder.feed(black_box(chunk), &mut append).unwrap();
                            }
                            decoder.finish(&mut append).unwrap();
                            black_box(append.len());
                        },
                        iters,
                    ),
                };
                values.push(ns);
            }
        }

        for (name, values) in &mut ops {
            let med = median(values);
            let bytes = size;
            println!("{name},{size},{med:.3},{:.3}", bytes as f64 / med);
            csv.push_str(&format!(
                "{name},{size},{med:.3},{:.3}\n",
                bytes as f64 / med
            ));
        }
    }
    let results = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&results).unwrap();
    std::fs::write(results.join("api.csv"), csv).unwrap();
    eprintln!("wrote {}", results.display());
}
