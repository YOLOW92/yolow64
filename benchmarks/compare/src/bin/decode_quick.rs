//! Focused warm decode benchmark for iterating on the bulk kernel.
//!
//! ```sh
//! cargo build --release --manifest-path benchmarks/compare/Cargo.toml \
//!   --bin decode_quick
//! taskset -c 4 benchmarks/compare/target/release/decode_quick
//! ```

use std::hint::black_box;
use std::time::Instant;

use base64::Engine as _;

struct Aligned {
    mem: Vec<u8>,
    start: usize,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let mem = vec![0u8; len + 64];
        let start = (64 - mem.as_ptr() as usize % 64) % 64;
        Self { mem, start }
    }

    fn slice(&self, len: usize) -> &[u8] {
        &self.mem[self.start..self.start + len]
    }

    fn slice_mut(&mut self, len: usize) -> &mut [u8] {
        &mut self.mem[self.start..self.start + len]
    }
}

const SIZES: &[usize] = &[8192, 16384, 65536, 262144, 1048576, 4194304];

fn iterations(size: usize) -> usize {
    (8 * 1024 * 1024 / size).clamp(16, 200_000)
}

fn main() {
    // Raise the core clock before the first, smallest size. Without this the
    // 8 KiB cell starts at the idle frequency and is not comparable with the
    // eight-library harness, which runs after the short and medium groups.
    {
        let warm_raw = vec![0x5au8; 1 << 20];
        let warm_encoded = base64::engine::general_purpose::STANDARD
            .encode(&warm_raw)
            .into_bytes();
        let mut warm_out = vec![0u8; warm_raw.len()];
        let warm_iters = 2000;
        for _ in 0..warm_iters {
            black_box(
                yolow64::STANDARD
                    .decode(black_box(&warm_encoded), black_box(&mut warm_out))
                    .unwrap(),
            );
        }
    }
    for &size in SIZES {
        let raw: Vec<u8> = (0..size).map(|i| (i * 73 + 19) as u8).collect();
        let encoded_bytes = base64::engine::general_purpose::STANDARD
            .encode(&raw)
            .into_bytes();
        let mut encoded_storage = Aligned::new(encoded_bytes.len());
        encoded_storage.mem[encoded_storage.start..encoded_storage.start + encoded_bytes.len()]
            .copy_from_slice(&encoded_bytes);
        let encoded = encoded_storage.slice(encoded_bytes.len());
        let mut output_storage = Aligned::new(size);
        let output = output_storage.slice_mut(size);
        let iters = iterations(size);
        let mut samples = Vec::with_capacity(9);
        for _ in 0..9 {
            let start = Instant::now();
            for _ in 0..iters {
                let written = yolow64::STANDARD
                    .decode(black_box(encoded), black_box(&mut *output))
                    .unwrap();
                black_box(written);
            }
            samples.push(start.elapsed().as_secs_f64() * 1e9 / iters as f64);
        }
        samples.sort_by(f64::total_cmp);
        let ns = samples[4];
        println!("{size},{ns:.2},{:.3}", size as f64 / ns);
    }
}
