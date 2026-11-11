use std::hint::black_box;
use std::time::Instant;

const CODEC: yolow64::Codec = yolow64::STANDARD;

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

#[cfg(turbo_base64)]
fn main() {
    let sizes = [4096usize, 8192, 16384, 65536, 262144, 1048576, 4194304];
    println!("size,yolow64_ns,turbo_ns,ratio");
    for &size in &sizes {
        let mut input = vec![0u8; size];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = CODEC.encoded_len(size).unwrap();
        let mut out_y = vec![0u8; encoded];
        let mut out_t = vec![0u8; encoded];
        let iters = (32 * 1024 * 1024 / size).clamp(8, 200_000);
        let mut ys = Vec::new();
        let mut ts = Vec::new();
        for round in 0..15 {
            for step in 0..2 {
                let which = (step + round) % 2;
                if which == 0 {
                    let ns = measure(
                        || {
                            black_box(
                                CODEC
                                    .encode(black_box(&input), black_box(&mut out_y))
                                    .unwrap(),
                            );
                        },
                        iters,
                    );
                    ys.push(ns);
                } else {
                    let ns = measure(
                        || {
                            black_box(unsafe {
                                tb64enc(
                                    black_box(input.as_ptr()),
                                    input.len(),
                                    black_box(out_t.as_mut_ptr()),
                                )
                            });
                        },
                        iters,
                    );
                    ts.push(ns);
                }
            }
        }
        let y = median(&mut ys);
        let t = median(&mut ts);
        println!("{size},{y:.3},{t:.3},{:.4}", y / t);
    }
}

#[cfg(turbo_base64)]
unsafe extern "C" {
    fn tb64enc(in_: *const u8, inlen: usize, out: *mut u8) -> usize;
}

#[cfg(not(turbo_base64))]
fn main() {
    eprintln!("Turbo-Base64 is not available; set TURBO_BASE64_DIR");
}
