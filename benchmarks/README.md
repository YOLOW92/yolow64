# Comparison method

Eight implementations are measured in one process on the same inputs. A broader 22-implementation encode-only comparison is in [`../research/encode-comparison.md`](../research/encode-comparison.md); counter-level analysis (instructions, cache misses and crossover points) is in [`../research/encode-counters.md`](../research/encode-counters.md). The decode round status, cold-buffer data and rejected kernels are in [`../research/decode.md`](../research/decode.md). The benchmark ran on 2026-11-11 against the source in this repository at its current revision.

## Implementations

| Name | Crate / source | Configuration |
|---|---|---|
| yolow64 | this repository | `STANDARD`, slice APIs |
| base64-scalar | [`base64`](https://crates.io/crates/base64) 0.23.1 | `GeneralPurpose` (default features disabled for this engine) |
| base64-simd-engine | `base64` 0.23.1 | `Simd` engine, runtime AVX2 |
| base64-simd | [`base64-simd`](https://crates.io/crates/base64-simd) 0.8.0 | `STANDARD` |
| base64-turbo | [`base64-turbo`](https://crates.io/crates/base64-turbo) 0.6.0 | `STANDARD` |
| encodify | [`encodify`](https://crates.io/crates/encodify) 1.0.2 | `base64::STANDARD`; `base64::MIME` for CRLF |
| simdutf | [`simdutf`](https://crates.io/crates/simdutf) 0.7.0 (C++ binding) | `Base64Options::Default`, `LastChunkHandlingOptions::Loose` |
| Turbo-Base64 | [`powturbo/Turbo-Base64`](https://github.com/powturbo/Turbo-Base64) | `tb64enc` / `tb64dec`, runtime AVX2 (GPL-3.0) |

Turbo-Base64 is GPL-3.0 and is **not** linked into the yolow64 library. It is built only into this local comparison binary. The other crates are used only as benchmark dependencies.

## Environment

- Intel Core i7-10700 (Comet Lake), Linux x86-64, AVX2/BMI2, no AVX-512.
- Rust 1.99.0, release profile, fat LTO, one codegen unit.
- Pinned to CPU 4 with `taskset`; the machine is shared and `schedutil` is active, so run-to-run noise is not negligible.
- 64-byte aligned, preallocated, reused input and output buffers.
- Turbo-Base64 is compiled by its own makefile via `TURBO_BASE64_DIR` (default `/tmp/Turbo-Base64`).

## Workload and statistics

Three complete independent processes were run for the eight-library table. Each process measures seven rounds per cell, rotating the implementation order every round; the reported cell is the median of the three process medians. `results/8way_combined.csv` is the three-process median used for the external columns. The `yolow64` decode rows are six-process medians: `results/8way_decode.csv` for large inputs and `results/8way_short_decode.csv` for short inputs. `results/8way_short_summary.csv` and `results/8way_short_raw.csv` are the recorded short-input run; only the `yolow64` rows are replaced by the six-process medians, and the external rows were not re-ranked.

Inputs are deterministic: byte `i` is `(i * 73 + 19) as u8`. Short and medium decode inputs are the valid padded encodings of those bytes. CRLF inputs wrap the standard encoding at 76 columns and append CRLF after every chunk, including the final one.

The machine is shared, so especially at 4 MiB the numbers are noisy. The `yolow64` decode row is post hash-loop; external columns are the recorded run and were not re-ranked. Turbo-Base64's large decode column is shown with `FULLCHECK=1` for like-for-like validation; the short and medium columns are the recorded default partial-check run. Its default build checks only the first of four blocks in each 256-byte DS128 hot-loop group (25%), while `FULLCHECK=1` checks every block; the build flags, dispatch target and disassembly counts are verified in [`../research/turbo-check.md`](../research/turbo-check.md). The experiment log with rejected kernels and paired comparisons is in [`research/experiments.md`](../research/experiments.md). The 8–128 B and 64 KiB rows are the most stable. Treat small differences at 1–4 MiB as inconclusive; the raw CSV is kept so the measurement can be re-analysed.

Charts: [decode across all eight libraries](../docs/figures/decode-all.svg), [short decode](../docs/figures/short-all.svg), [recorded 22-way encode](../docs/figures/encode-all.svg). The all-library medians behind the decode figures are in [`results/8way_decode_all.csv`](results/8way_decode_all.csv) and [`results/8way_short_all.csv`](results/8way_short_all.csv).

## Results

### Short input latency, ns/call (lower is better)

Encode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (default) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 8 | 9.28 | 9.61 | 10.95 | 7.23 | 7.83 | 16.73 | 11.62 | 7.01 |
| 12 | 7.33 | 11.58 | 13.11 | 9.18 | 7.69 | 10.28 | 13.57 | 7.69 |
| 16 | 10.27 | 14.65 | 15.94 | 7.58 | 9.89 | 12.84 | 15.98 | 8.76 |
| 24 | 9.25 | 19.02 | 20.64 | 10.71 | 10.48 | 9.76 | 20.12 | 10.54 |
| 32 | 12.98 | 19.02 | 20.45 | 13.77 | 13.00 | 13.01 | 13.79 | 9.40 |
| 48 | 10.40 | 28.52 | 29.93 | 13.37 | 18.90 | 10.79 | 22.18 | 12.90 |
| 64 | 13.62 | 33.56 | 34.78 | 15.07 | 19.26 | 13.59 | 19.37 | 12.24 |
| 96 | 12.36 | 47.50 | 48.87 | 17.81 | 21.29 | 13.71 | 24.54 | 14.93 |
| 128 | 17.51 | 56.80 | 23.23 | 22.45 | 21.16 | 17.65 | 19.00 | 13.91 |

After the 8/16/32-byte single-load paths, the `yolow64` encode row is: 8 B 6.02, 12 B 7.35, 16 B 6.20, 24 B 9.01, 32 B 7.29, 48 B 9.95, 64 B 13.41, 96 B 11.43 and 128 B 17.40 ns. The other columns are the recorded 2026-11-11 run and were not re-ranked.

Decode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (default) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 8 | **5.47** | 19.00 | 19.54 | 7.92 | 8.32 | 19.24 | 26.76 | 7.90 |
| 12 | **5.04** | 20.97 | 21.62 | 9.46 | 10.70 | 13.28 | 26.40 | 8.78 |
| 16 | **5.69** | 23.61 | 24.07 | 11.81 | 11.93 | 15.94 | 29.99 | 8.58 |
| 24 | **7.87** | 27.50 | 28.00 | 14.65 | 14.43 | 12.70 | 31.33 | 10.62 |
| 32 | **6.89** | 30.56 | 31.44 | 10.24 | 13.60 | 15.82 | 36.47 | 10.78 |
| 48 | **9.69** | 38.80 | 39.74 | 19.77 | 19.37 | 14.20 | 20.82 | 13.06 |
| 64 | **10.96** | 46.93 | 30.21 | 18.43 | 18.15 | 16.48 | 35.27 | 12.79 |
| 96 | **13.02** | 62.08 | 35.35 | 25.04 | 22.04 | 17.62 | 23.84 | 16.81 |
| 128 | **14.24** | 76.17 | 29.43 | 22.81 | 18.79 | 20.09 | 44.50 | 19.20 |

The `yolow64` decode row is the six-process median after the v0.3.0 dedicated short kernels (`decode12`, `decode24`, `decode44`, `decode88`) and fused `decode_hash4_tail44`; it is **first in all 9 short sizes (8–128 B)**. External columns are the recorded 2026-11-11 run and were not re-ranked.

### Medium input throughput, GB/s (higher is better)

Encode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (default) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 256 | 11.2 | 2.4 | 6.5 | 7.9 | 8.8 | 11.4 | 8.5 | 11.6 |
| 512 | 15.0 | 2.5 | 8.6 | 9.0 | 12.3 | 14.7 | 13.2 | 15.7 |
| 1024 | 18.2 | 2.5 | 9.0 | 10.1 | 15.8 | 18.1 | 15.1 | 19.2 |
| 4096 | 21.8 | 2.5 | 9.9 | 10.7 | 20.3 | 21.3 | 18.7 | 23.2 |

Decode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (default) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 256 | 8.6 | 1.9 | 5.7 | 6.1 | 9.0 | 10.1 | 5.6 | 12.4 |
| 512 | 11.5 | 2.0 | 8.6 | 7.3 | 13.4 | 13.2 | 7.8 | 15.0 |
| 1024 | 14.5 | 2.0 | 9.8 | 7.5 | 15.2 | 16.3 | 11.5 | 22.6 |
| 4096 | 16.1 | 2.1 | 11.3 | 7.6 | 17.1 | 17.6 | 14.3 | 26.1 |

### Large input throughput, GB/s (higher is better)

Encode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (default) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 8192 | 21.9 | 2.5 | 10.0 | 10.8 | 21.0 | 21.5 | 19.4 | 23.7 |
| 65536 | 22.5 | 2.5 | 10.0 | 10.7 | 20.9 | 21.4 | 18.7 | 23.7 |
| 1048576 | 22.3 | 2.5 | 9.5 | 10.4 | 19.4 | 19.4 | 17.1 | 21.0 |
| 4194304 | 17.7 | 2.4 | 8.9 | 9.5 | 15.7 | 15.8 | 14.2 | 16.8 |

Decode:

| Input | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 (full) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 8192 | **20.4** | 2.1 | 12.4 | 8.1 | 19.0 | 19.7 | 16.0 | 20.1 |
| 65536 | **20.4** | 2.1 | 12.3 | 7.9 | 18.0 | 18.5 | 14.5 | 19.4 |
| 1048576 | **20.4** | 2.1 | 11.8 | 7.8 | 16.5 | 16.6 | 13.4 | 16.7 |
| 4194304 | **17.1** | 2.0 | 8.8 | 6.4 | 13.4 | 13.5 | 10.1 | 13.9 |

### CRLF-tolerant decode, GB/s (higher is better)

The libraries do not accept the same inputs. `native` means the library's own whitespace/MIME decoder; `filter+strict` means the benchmark filters CRLF into a reused buffer and then calls the strict decoder, including the filter time.

| Input | yolow64 (native) | base64-scalar (filter+strict) | base64-simd-engine (filter+strict) | base64-simd (native) | base64-turbo (filter+strict) | encodify (native) | simdutf (native) | Turbo-Base64 (filter+strict) |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| 4096 | 7.3 | 0.8 | 1.1 | 1.0 | 1.1 | 10.0 | 7.9 | 1.2 |
| 65536 | 9.4 | 0.8 | 1.1 | 1.1 | 1.2 | 10.4 | 8.1 | 1.2 |
| 1048576 | 8.6 | 0.8 | 1.1 | 1.0 | 1.1 | 10.1 | 7.9 | 1.2 |
| 4194304 | 6.4 | 0.7 | 1.0 | 1.0 | 1.0 | 7.6 | 7.1 | 1.1 |

## Owned and streaming APIs

`benchmarks/compare/src/bin/api.rs` measures API overhead at 64 KiB, 1 MiB and 4 MiB. It includes `encode_slice`, `encode_to_string`, `append_encode`, `decode_slice`, `decode_to_vec`, `append_decode`, one-shot streaming, and 4 KiB streaming. Results are written to [`results/api.csv`](results/api.csv). The current implementation writes into spare `Vec` capacity instead of zero-filling it first, so owned and append APIs track the slice APIs closely at 64 KiB and 1 MiB.

## Cold-buffer one-shot benchmark

`benchmarks/compare/src/bin/encode_cold.rs` rotates through a 64 MiB arena instead of reusing one input/output pair. It measures warm `encode`, cold `encode`, and cold `encode_streaming` at 4 KiB–16 MiB. Cold `encode` falls to 5.7–8.0 GB/s from 4 KiB to 4 MiB; `encode_streaming` uses non-temporal stores from 4 KiB and recovers 7.9–13.4 GB/s (1.39–1.68x). Three-process medians are in [`results/encode_cold_combined.csv`](results/encode_cold_combined.csv); `encode_cold_summary.csv` and `encode_cold_raw.csv` are from the last process.

`benchmarks/compare/src/bin/encode_memory.rs` measures peak RSS for `encode_to_string` versus `encode_to_writer`. At 256 MiB input the writer path peaks at about 258 MiB RSS versus 599 MiB for the owned string; [`results/encode_memory.csv`](results/encode_memory.csv) has the exact values.

## Adaptive parallel API

The crate has an optional `parallel` feature:

```toml
[dependencies]
yolow64 = { git = "...", features = ["parallel"] }
```

`encode_parallel` / `decode_parallel` choose the worker count from the input size and the detected last-level cache. Encode stays single-threaded below 4 MiB; from 4 MiB up to the LLC working-set limit it uses up to four workers; above that the single-thread streaming-store path is faster. Decode still uses at most two workers and a 7/8-LLC margin until its own crossover round. `encode_parallel_with_threads` / `decode_parallel_with_threads` let the caller force a worker cap for experiments.

`benchmarks/compare/src/bin/parallel_encode.rs` measures 2–32 MiB with 1/2/4 workers and the automatic policy. On this i7-10700, four workers improve encode by about 20–25% from 4 to 6.5 MiB; 7 MiB and above are memory-bandwidth-bound and stay single-threaded. `parallel_one.rs` measures one size/thread count per process. These thresholds should be re-measured on other hardware; the policy reads the LLC from CPUID on x86_64 and from sysfs on other Linux architectures.

## Reproduce

```sh
# Optional; the build script defaults to /tmp/Turbo-Base64.
git clone --depth 1 https://github.com/powturbo/Turbo-Base64.git /tmp/Turbo-Base64
export TURBO_BASE64_DIR=/tmp/Turbo-Base64

cargo build --release --manifest-path benchmarks/compare/Cargo.toml
for i in 1 2 3; do
  taskset -c 4 benchmarks/compare/target/release/yolow64-compare
  cp benchmarks/results/8way_summary.csv "/tmp/8way_$i.csv"
done

taskset -c 4 benchmarks/compare/target/release/api
# Cold-buffer one-shot encode:
taskset -c 4 benchmarks/compare/target/release/encode_cold
# Peak RSS, owned versus writer:
/usr/bin/time -f 'max_rss_kb=%M' benchmarks/compare/target/release/encode_memory string 256
/usr/bin/time -f 'max_rss_kb=%M' benchmarks/compare/target/release/encode_memory writer 256
# Parallel scaling across sizes and worker counts:
taskset -c 4-7 benchmarks/compare/target/release/parallel_encode
# One process per configuration:
taskset -c 4 benchmarks/compare/target/release/parallel_one 4194304 2 encode 15
```

`taskset` is Linux-specific; choose an available CPU and avoid concurrent benchmarks. The exact Turbo-Base64 build is GPL-3.0; keep it outside the yolow64 repository and outside released binaries.
