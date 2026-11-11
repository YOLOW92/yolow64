use std::hint::black_box;
use std::time::Instant;

const ROUNDS: usize = 7;
const SIZES: &[usize] = &[
    1024 * 1024,
    2 * 1024 * 1024,
    4 * 1024 * 1024,
    8 * 1024 * 1024,
    16 * 1024 * 1024,
];
const THREADS: &[usize] = &[1, 2, 4, 8];

const CODEC: yolow64::Codec = yolow64::STANDARD;

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

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}

fn measure<F: FnMut()>(mut f: F, iters: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iters {
        f();
    }
    start.elapsed().as_secs_f64() * 1e9 / iters as f64
}

fn main() {
    println!("op,size,threads,median_ns,gbps,speedup");
    let mut csv = String::from("op,size,threads,median_ns,gbps,speedup\n");

    for &size in SIZES {
        let mut input = Aligned::new(size);
        for (i, b) in input.slice_mut().iter_mut().enumerate() {
            *b = (i * 73 + 19) as u8;
        }
        let encoded = CODEC.encode_to_string(input.slice()).unwrap();
        let encoded_len = encoded.len();
        let mut encoded_buf = Aligned::new(encoded_len);
        encoded_buf.slice_mut().copy_from_slice(encoded.as_bytes());
        let mut output = Aligned::new(encoded_len.max(size) + 64);
        let iters = (16 * 1024 * 1024 / size).clamp(1, 16);

        let mut encodes: Vec<Vec<f64>> = THREADS.iter().map(|_| Vec::new()).collect();
        let mut decodes: Vec<Vec<f64>> = THREADS.iter().map(|_| Vec::new()).collect();
        for round in 0..ROUNDS {
            for step in 0..THREADS.len() {
                let idx = (step + round) % THREADS.len();
                let threads = THREADS[idx];
                let enc = measure(
                    || {
                        black_box(
                            CODEC
                                .encode_parallel_with_threads(
                                    black_box(input.slice()),
                                    black_box(output.slice_mut()),
                                    threads,
                                )
                                .unwrap(),
                        );
                    },
                    iters,
                );
                encodes[idx].push(enc);
                let dec = measure(
                    || {
                        black_box(
                            CODEC
                                .decode_parallel_with_threads(
                                    black_box(encoded_buf.slice()),
                                    black_box(output.slice_mut()),
                                    threads,
                                )
                                .unwrap(),
                        );
                    },
                    iters,
                );
                decodes[idx].push(dec);
            }
        }

        let enc_meds: Vec<f64> = encodes.iter_mut().map(|v| median(v)).collect();
        let dec_meds: Vec<f64> = decodes.iter_mut().map(|v| median(v)).collect();
        let base_enc = enc_meds[0];
        let base_dec = dec_meds[0];
        for (idx, &threads) in THREADS.iter().enumerate() {
            let enc = enc_meds[idx];
            let dec = dec_meds[idx];
            let enc_speedup = base_enc / enc;
            let dec_speedup = base_dec / dec;
            println!(
                "encode,{size},{threads},{enc:.3},{:.3},{enc_speedup:.3}",
                size as f64 / enc
            );
            println!(
                "decode,{size},{threads},{dec:.3},{:.3},{dec_speedup:.3}",
                size as f64 / dec
            );
            csv.push_str(&format!(
                "encode,{size},{threads},{enc:.3},{:.3},{enc_speedup:.3}\n",
                size as f64 / enc
            ));
            csv.push_str(&format!(
                "decode,{size},{threads},{dec:.3},{:.3},{dec_speedup:.3}\n",
                size as f64 / dec
            ));
        }
    }

    let results = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&results).unwrap();
    std::fs::write(results.join("parallel.csv"), csv).unwrap();
    eprintln!("wrote {}", results.display());
}
