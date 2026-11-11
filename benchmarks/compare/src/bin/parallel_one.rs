use std::hint::black_box;
use std::time::Instant;

const CODEC: yolow64::Codec = yolow64::STANDARD;

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn main() {
    let mut args = std::env::args().skip(1);
    let size: usize = args.next().expect("size").parse().unwrap();
    let threads: usize = args.next().expect("threads").parse().unwrap();
    let op = args.next().expect("encode|decode");
    let rounds: usize = args.next().map_or(15, |s| s.parse().unwrap());
    let iters = (16 * 1024 * 1024 / size).clamp(1, 16);

    let mut input = vec![0u8; size];
    for (i, b) in input.iter_mut().enumerate() {
        *b = (i * 73 + 19) as u8;
    }
    let encoded = CODEC.encode_to_string(&input).unwrap();
    let mut output = vec![0u8; encoded.len().max(size) + 64];

    let mut times = Vec::with_capacity(rounds);
    for _ in 0..rounds {
        let start = Instant::now();
        for _ in 0..iters {
            match op.as_str() {
                "encode" => {
                    let n = if threads == 0 {
                        CODEC
                            .encode_parallel(black_box(&input), black_box(&mut output))
                            .unwrap()
                    } else {
                        CODEC
                            .encode_parallel_with_threads(
                                black_box(&input),
                                black_box(&mut output),
                                threads,
                            )
                            .unwrap()
                    };
                    black_box(n);
                }
                "decode" => {
                    let n = if threads == 0 {
                        CODEC
                            .decode_parallel(black_box(encoded.as_bytes()), black_box(&mut output))
                            .unwrap()
                    } else {
                        CODEC
                            .decode_parallel_with_threads(
                                black_box(encoded.as_bytes()),
                                black_box(&mut output),
                                threads,
                            )
                            .unwrap()
                    };
                    black_box(n);
                }
                _ => panic!("op"),
            }
        }
        times.push(start.elapsed().as_secs_f64() * 1e9 / iters as f64);
    }
    let med = median(&mut times);
    println!("{op},{size},{threads},{med:.3},{:.3}", size as f64 / med);
}
