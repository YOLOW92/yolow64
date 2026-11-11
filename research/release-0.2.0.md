# Release 0.2.0 benchmark and verification record

Date: 2026-11-11. Machine: Intel Core i7-10700 (Comet Lake), 16 GiB, AVX2 and
BMI2, no AVX-512, Linux x86-64, Rust 1.99.0, release fat LTO. The machine is
shared; use these as snapshots, not hardware-independent guarantees.

## Build

```sh
cargo build --release --manifest-path benchmarks/compare/Cargo.toml \
  --bin yolow64-compare
taskset -c 4 benchmarks/compare/target/release/yolow64-compare
```

Public figures: [decode throughput](../docs/figures/decode-all.svg),
[short decode](../docs/figures/short-all.svg),
[encode throughput](../docs/figures/encode-all.svg),
[decode pipeline](../docs/figures/decode-pipeline.svg),
[CPU dispatch](../docs/figures/cpu-dispatch.svg),
[encode write policy](../docs/figures/encode-streaming.svg).

The six-process medians for decode are in
[`../benchmarks/results/8way_decode.csv`](../benchmarks/results/8way_decode.csv)
and
[`../benchmarks/results/8way_short_decode.csv`](../benchmarks/results/8way_short_decode.csv).
The final six-process encode medians are in
[`../benchmarks/results/8way_encode_final.csv`](../benchmarks/results/8way_encode_final.csv).

## Decode

| Input | yolow64 | FULLCHECK Turbo | Result |
|---:|---:|---:|---|
| 16 KiB | 19.54 | 19.68 | −0.7%, tie |
| 64 KiB | 20.08 | 19.36 | +3.7% |
| 256 KiB | 20.16 | 16.76 | +20.3% |
| 1 MiB | 20.17 | 16.68 | +20.9% |
| 4 MiB | 17.06 | 13.89 | +22.8% |

Short decode, ns/call:

| Input | 8 B | 12 B | 16 B | 24 B | 32 B | 48 B | 64 B | 96 B | 128 B |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| yolow64 | 14.26 | **7.00** | **8.53** | **10.14** | 19.14 | **12.20** | 23.18 | **15.68** | **17.18** |

First at 12, 16, 24, 48, 96 and 128 B.

## Encode regression check

Six-process medians from the final eight-library build. The recorded baseline
is `results/8way_combined.csv` (the pre-decode-round run; it already includes
the earlier encode fast paths). Positive deltas are improvements.

| Input | Baseline GB/s | Final GB/s | Change |
|---:|---:|---:|---:|
| 8 KiB | 21.9 | 21.9 | flat |
| 16 KiB | 22.3 | 22.4 | +0.5% |
| 64 KiB | 22.5 | 22.5 | flat |
| 256 KiB | 21.6 | 22.2 | +2.8% |
| 1 MiB | 21.7 | 22.3 | +2.6% |
| 4 MiB | 15.5 | 17.7 | +14.5% |

The 16 MiB streaming path was rechecked with `encode_cold` in the final build:
warm `encode` 13.05 GB/s, warm `encode_streaming` 13.03, cold `encode` 11.68,
cold `encode_streaming` 11.60. This is within the recorded 13.0–13.3 GB/s
range; no encode regression.

## Correctness and safety checks

- `cargo test --release --locked --all-features`: unit, scalar, parallel and
  integration tests pass.
- Standard and URL-safe, padded and no-pad, all byte mutations and
  every invalid-byte position are covered by the scalar suite.
- Non-zero trailing bits are rejected at the short padded lengths and at the
  generic tail; new tests cover Standard and URL-safe.
- Exact-output canary tests cover caller-provided decode buffers at 1 B–8 KiB
  and streaming encode output at 4 KiB–1 MiB. The AVX2 store spill, `pext`
  tails and non-temporal stores do not write past the exact output.
- A test-only thread-local switch forces the no-BMI2 fallback on a BMI2 CPU;
  24/44/172-character padded inputs still decode identically and invalid input
  still errors. AVX2 and BMI2 are detected independently in production code.
- `cargo clippy --all-targets --all-features` is clean; MSRV Rust 1.89 check,
  aarch64-apple-darwin and x86_64-pc-windows-gnu cross-checks pass.

## Validation-parity note

Turbo-Base64's default build enables `CHECK0` but leaves `CHECK1` empty. In the
256-byte hot loop it checks the first of four 32-byte blocks (25%). The tables
above use `FULLCHECK=1`, which adds `-DB64CHECK` and checks every block. Build
and disassembly verification is in
[`turbo-check.md`](turbo-check.md); raw comparison data is in
[`../benchmarks/results/turbo_check_verify.csv`](../benchmarks/results/turbo_check_verify.csv).
