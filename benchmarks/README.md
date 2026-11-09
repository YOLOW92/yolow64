# Comparison method

This comparison was run on 2026-11-09 against `base64` 0.23.1 and `base64-simd` 0.8.0. Times are microseconds per call; lower is better. The exact implementation under test is the source in this repository prior to documentation and example changes; it was measured after the crate was renamed to `yolow64`.

## Environment

- Intel Core i7-10700, Linux x86-64, AVX2, CPU 0.
- Rust 1.99.0, LLVM 23.1.1, release profile, fat LTO and one codegen unit. No extra `target-cpu` flags.
- One process, 64-byte aligned input and output buffers, preallocated and reused; repeated access measures a warm working set, not all alignments or cold-cache behavior.

## Workload and statistics

For an original size `n`, byte `i` is `(i * 73 + 19) as u8`. Strict tests use the standard alphabet and padding. CRLF tests insert CRLF after every 76 encoded characters, including the final chunk. The encoded/decoded data are checked before timing.

Each round measures all six combinations (three libraries, encode and decode); ordering rotates and alternates forward/reverse. Each operation has `(16 MiB / n).clamp(32, 500_000)` iterations per round. The program reports the median of seven rounds. README cells are the median of the corresponding cell in three complete runs; columns are independent and can come from different runs. The CSVs retain each run's seven-round median, not the individual round timings.

For CRLF input, `base64` is measured with a separate four-character whitespace filter into a reused buffer followed by `decode_slice`; `base64-simd` uses `forgiving_decode`. yolow64 and `base64-simd` do not accept exactly the same inputs or return the same errors. This benchmark compares valid padded CRLF inputs, not full semantic equivalence. `base64-simd`'s forgiving API requires output capacity at least as large as its encoded input, so all three share an output buffer large enough for that requirement.

`black_box` is applied to input/output buffers. Normal CPU scheduling, frequency changes and other processes affect timings. Short-input differences are particularly noise-sensitive. The `mode` does not change encoding; its encoding columns are repeated measurements.

## Reproduce

From the repository root:

```sh
cargo build --release --locked --manifest-path benchmarks/compare/Cargo.toml
for i in 1 2 3; do
  taskset -c 0 benchmarks/compare/target/release/yolow64-compare > "run_${i}.csv"
done
```

`taskset` is Linux-specific; choose an available CPU and avoid concurrent benchmarks. This writes to the current directory, not over the committed results. The isolated benchmark workspace's `Cargo.lock` pins the comparison dependencies. The timing loop is in [`compare/src/main.rs`](compare/src/main.rs).

## Results

- [Run 1](results/run_1.csv)
- [Run 2](results/run_2.csv)
- [Run 3](results/run_3.csv)
- [Complete median tables](../README.md#performance)
