//! Cold-buffer encode benchmark: rotate through a working set larger than the
//! last-level cache instead of reusing one warm input/output pair.
//!
//! ```sh
//! cargo build --release --manifest-path benchmarks/compare/Cargo.toml \
//!   --bin encode_cold
//! taskset -c 4 benchmarks/compare/target/release/encode_cold
//! ```

use std::hint::black_box;
use std::time::Instant;

const SIZES: &[usize] = &[
    4 * 1024,
    16 * 1024,
    64 * 1024,
    256 * 1024,
    1024 * 1024,
    4 * 1024 * 1024,
    16 * 1024 * 1024,
];
const ROUNDS: usize = 5;
const TARGET_WORKING_SET: usize = 64 * 1024 * 1024;

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

struct Arena {
    data: Vec<u8>,
    stride: usize,
    len: usize,
    start: usize,
}

impl Arena {
    fn new(len: usize, count: usize) -> Self {
        let stride = align_up(len, 64);
        let mut data = vec![0u8; count * stride + 64];
        let start = (64 - data.as_ptr() as usize % 64) % 64;
        data.fill(0);
        Self {
            data,
            stride,
            len,
            start,
        }
    }

    fn slice(&self, slot: usize) -> &[u8] {
        let base = self.start + slot * self.stride;
        &self.data[base..base + self.len]
    }

    fn slice_mut(&mut self, slot: usize) -> &mut [u8] {
        let base = self.start + slot * self.stride;
        &mut self.data[base..base + self.len]
    }
}

fn main() {
    let mut raw = String::from("size,mode,round,ns,gbps,tsc_cycles_per_byte\n");
    let mut summary = String::from("size,mode,median_ns,gbps,tsc_cycles_per_byte\n");

    for &size in SIZES {
        let cap = yolow64::STANDARD.encoded_len(size).unwrap();
        let stride = align_up(size, 64) + align_up(cap, 64);
        let count = (TARGET_WORKING_SET / stride).max(2);
        let mut input = Arena::new(size, count);
        let mut output = Arena::new(cap, count);
        for slot in 0..count {
            for (i, byte) in input.slice_mut(slot).iter_mut().enumerate() {
                *byte = ((slot * 131 + i * 73 + 19) & 0xff) as u8;
            }
        }
        // Fault in every output page before timing.
        for slot in 0..count {
            output.slice_mut(slot).fill(0);
        }

        let mut reference = vec![0u8; cap];
        let n = yolow64::STANDARD
            .encode(input.slice(0), &mut reference)
            .unwrap();
        assert_eq!(n, cap);
        drop(reference);

        // Scatter consecutive calls across the arena so the hardware
        // prefetcher cannot turn the rotation into one linear stream.
        let mut step = 7919 % count;
        while step > 1 && gcd(step, count) != 1 {
            step -= 1;
        }
        let step = step.max(1);
        let iters = (32 * 1024 * 1024 / size).clamp(4, 8192);

        for (mode, streaming) in [
            ("warm", false),
            ("cold", false),
            ("warm-stream", true),
            ("cold-stream", true),
        ] {
            let mut ns_samples = Vec::with_capacity(ROUNDS);
            let mut cycle_samples = Vec::with_capacity(ROUNDS);
            for round in 0..ROUNDS {
                let mut slot = if mode.starts_with("warm") {
                    0
                } else {
                    round % count
                };
                let start_t = Instant::now();
                // SAFETY: `_rdtsc` is available on x86_64.
                let start_c = unsafe { std::arch::x86_64::_rdtsc() };
                for _ in 0..iters {
                    let written = if streaming {
                        yolow64::STANDARD
                            .encode_streaming(
                                black_box(input.slice(slot)),
                                black_box(output.slice_mut(slot)),
                            )
                            .unwrap()
                    } else {
                        yolow64::STANDARD
                            .encode(
                                black_box(input.slice(slot)),
                                black_box(output.slice_mut(slot)),
                            )
                            .unwrap()
                    };
                    black_box(written);
                    if mode.starts_with("cold") {
                        slot = (slot + step) % count;
                    }
                }
                // SAFETY: `_rdtsc` is available on x86_64.
                let end_c = unsafe { std::arch::x86_64::_rdtsc() };
                let ns = start_t.elapsed().as_secs_f64() * 1e9 / iters as f64;
                let tsc = (end_c.wrapping_sub(start_c)) as f64 / iters as f64 / size as f64;
                ns_samples.push(ns);
                cycle_samples.push(tsc);
                raw.push_str(&format!(
                    "{size},{mode},{round},{ns:.4},{:.4},{tsc:.6}\n",
                    size as f64 / ns
                ));
            }
            ns_samples.sort_by(f64::total_cmp);
            cycle_samples.sort_by(f64::total_cmp);
            let ns = ns_samples[ROUNDS / 2];
            let tsc = cycle_samples[ROUNDS / 2];
            summary.push_str(&format!(
                "{size},{mode},{ns:.4},{:.4},{tsc:.6}\n",
                size as f64 / ns
            ));
        }
        eprintln!("size={size} buffers={count} step={step}");
    }

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../results");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("encode_cold_raw.csv"), raw).unwrap();
    std::fs::write(dir.join("encode_cold_summary.csv"), summary).unwrap();
    eprintln!("wrote {}", dir.display());
}
