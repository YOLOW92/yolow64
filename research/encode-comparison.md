# Encode cross-comparison: 22 implementations

This is a detailed, encode-only horizontal comparison across Rust, C and C++ Base64 implementations. Measurement date: 2026-11-11. The library under development is `yolow64`; the other projects are used only as benchmark dependencies or isolated benchmark binaries.

Raw data: [`../benchmarks/results/encode_all_raw.csv`](../benchmarks/results/encode_all_raw.csv) (all rounds from the three processes, with a `process` column) and [`../benchmarks/results/encode_all_combined.csv`](../benchmarks/results/encode_all_combined.csv) (three-process medians).

Counter-level analysis (instructions, D1/LLd miss rates, crossover points) is in [`encode-counters.md`](encode-counters.md).

## Implementations

| Name | Version / source | Language | API used | Notes |
|---|---|---|---|---|
| yolow64 | this repository | Rust | `Codec::encode` | current experiment baseline |
| base64 (scalar) | `base64` 0.23.1 | Rust | `GeneralPurpose::encode_slice` | pure scalar engine |
| base64 (SIMD) | `base64` 0.23.1 | Rust | `Simd::standard` | runtime AVX2 |
| base64-simd | 0.8.0 | Rust | `STANDARD.encode` | AVX2/NEON runtime |
| base64-turbo | 0.6.0 | Rust | `STANDARD.encode_slice` | runtime AVX2 (no AVX-512 on this host) |
| encodify | 1.0.2 | Rust | `base64::STANDARD.encode_slice` | SIMD kernels, runtime detection |
| simdutf | 0.7.0 | C++ binding | `binary_to_base64` | upstream simdutf library |
| Turbo-Base64 | commit `d9e58436` | C | `tb64enc` | GPL-3.0, isolated benchmark only |
| aklomp/base64 | commit `5d5074cb` | C | `base64_encode(..., flags = 0)` | BSD-2-Clause; AVX2 build |
| fastbase64 (scalar) | commit `a2e0967d` | C | `scalar_base64_encode` | BSD-2-Clause |
| fastbase64 (chromium) | same | C | `chromium_base64_encode` | BSD-2-Clause |
| fastbase64 (klomp AVX2) | same | C | `klomp_avx2_base64_encode` | BSD-2-Clause |
| fastbase64 (fast AVX2) | same | C | `fast_avx2_base64_encode` | BSD-2-Clause |
| base64-ng | 2.1.0, `simd` feature | Rust | `STANDARD.encode_slice` | optional SIMD enabled |
| data-encoding | 2.11.1 | Rust | `BASE64.encode_mut` | scalar, table-driven |
| base64ct | 1.8.3 | Rust | `Base64::encode` | constant-time scalar |
| base64_light | 0.1.6 | Rust | `base64_encode_bytes` | returns `String`; allocating API |
| simple-base64 | 0.23.2 | Rust | `encode_engine_slice` | scalar |
| tape-base64 | 0.1.0 | Rust | `STANDARD.encode` | slice API |
| radix64 | 0.6.2 | Rust | `STD.encode_slice` | SIMD feature default |
| fast32 | 1.0.3 | Rust | `base64::RFC4648.encode` | returns `String`; allocating API |
| base64-url | 4.0.1 | Rust | `encode_to_slice` | URL-safe no-pad only; URL table |

“Allocating API” means the crate has no caller-buffer encode entry point in the path we measured; the allocation is part of that row's real cost. All other rows write into a caller-provided buffer.

## Environment

- Intel Core i7-10700 (Comet Lake), 8 cores / 16 threads, 16 MiB L3, 2 MiB L2 per core, AVX2/BMI2, **no AVX-512**.
- Linux x86-64, pinned to CPU 4 with `taskset`. The machine is shared and `schedutil` is active.
- Rust 1.99.0, release profile, fat LTO, one codegen unit.
- GCC/G++ 15.2.0 for the C/C++ comparison libraries.
- Turbo-Base64, aklomp/base64 and fastbase64 are built from separate clones outside the repository. Turbo-Base64 is GPL-3.0 and is not linked into `yolow64`; the other two are BSD-2-Clause.

## Method

For every size, the input is a deterministic byte pattern `(i * 73 + 19) as u8`. The expected output is computed with the `base64` crate before timing, and every implementation is checked for byte equality before it is measured.

- Input and output buffers are 64-byte aligned and reused.
- Short sizes use a 1 MiB target per sample: `(1 MiB / n).clamp(10_000, 200_000)` calls.
- Medium and large sizes use an 8 MiB target per sample: `(8 MiB / n).clamp(8, 500_000)` calls.
- Each process runs seven rounds; the implementation order is rotated one position per round. The process median is the median of the seven round times.
- Three independent processes were run. The reported cell is the median of the three process medians.
- The table is produced by the same binary and the same input buffer for all libraries, rotating the order. This is a same-process, warm-cache comparison.
- No AVX-512 path is available on this host. C libraries were compiled with their own project flags, listed above.

### Short-call caveat

Short calls are sensitive to code layout and harness details. In the dedicated eight-library harness, `yolow64` encodes 12 bytes in about 7.3 ns; in this 22-library binary it is about 10 ns. The difference is larger than the per-call cost of most implementations and is not explained by the codec's algorithm alone: binary layout, inlining and code-cache pressure change short-call results. We keep the broad table for breadth, and the narrower eight-library table in [the main benchmark](../benchmarks/README.md) for the most stable short-input comparison. Treat sub-100 ns differences as approximate.

## Results

The tables below are the three-process medians. “ns” is lower-is-better; “GB/s” is input bytes per second, higher-is-better. `alloc` rows allocate their result.

### Short encode, ns/call

| Library | 8 | 12 | 16 | 24 | 32 | 48 | 64 | 96 | 128 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| Turbo-Base64 | 6.77 | 7.65 | 8.55 | 10.50 | 9.41 | 12.89 | 12.11 | 14.93 | 13.74 |
| aklomp-base64 | 13.15 | 17.05 | 21.43 | 29.29 | 16.71 | 32.86 | 26.68 | 34.63 | 21.06 |
| yolow64 | 10.19 | 9.99 | 13.28 | 10.45 | 14.50 | 11.41 | 15.33 | 13.67 | 20.34 |
| encodify | 17.84 | 10.95 | 13.82 | 10.58 | 14.50 | 11.48 | 15.82 | 14.14 | 20.25 |
| base64-turbo | 8.30 | 7.69 | 10.11 | 10.50 | 13.29 | 19.31 | 20.40 | 22.52 | 22.61 |
| tape-base64 | 15.54 | 8.85 | 14.85 | 8.91 | 14.90 | 11.17 | 17.75 | 13.42 | 19.04 |
| fastbase64-fast-avx2 | 8.74 | 10.71 | 13.56 | 17.95 | 10.92 | 21.63 | 18.15 | 23.28 | 16.60 |
| simdutf | 11.37 | 13.33 | 15.95 | 19.90 | 13.61 | 22.11 | 19.25 | 24.23 | 18.77 |
| fastbase64-klomp-avx2 | 7.86 | 10.49 | 13.32 | 18.78 | 9.76 | 20.63 | 18.50 | 26.81 | 15.53 |
| base64-ng | 40.99 | 143.63 | 164.60 | 157.91 | 196.65 | 159.61 | 228.41 | 153.81 | 192.82 |
| base64-simd | 7.00 | 8.95 | 8.79 | 10.66 | 13.57 | 13.23 | 21.96 | 16.89 | 21.82 |
| base64-simd-engine | 12.24 | 14.24 | 17.26 | 21.65 | 21.67 | 31.24 | 36.08 | 50.11 | 22.98 |
| radix64-std | 9.62 | 11.68 | 14.74 | 19.46 | 14.62 | 24.90 | 24.92 | 35.69 | 31.89 |
| simple-base64 | 9.19 | 11.02 | 14.10 | 18.21 | 18.57 | 27.78 | 32.87 | 46.66 | 56.09 |
| base64-scalar | 10.93 | 12.92 | 15.94 | 20.33 | 20.54 | 29.76 | 34.93 | 48.81 | 58.26 |
| data-encoding | 15.09 | 12.02 | 17.05 | 16.84 | 24.90 | 26.25 | 36.11 | 45.30 | 62.91 |
| fastbase64-scalar | 10.10 | 13.76 | 14.93 | 19.45 | 21.69 | 31.03 | 38.13 | 54.22 | 68.06 |
| base64ct | 18.08 | 17.09 | 26.94 | 29.10 | 42.07 | 57.03 | 53.07 | 78.08 | 93.68 |
| fastbase64-chromium | 7.58 | 9.57 | 12.48 | 16.80 | 21.91 | 31.23 | 41.37 | 64.66 | 83.59 |
| fast32-rfc4648 | 21.60 | 15.06 | 75.82 | 21.22 | 84.03 | 35.40 | 111.88 | 60.76 | 147.67 |
| base64_light | 27.49 | 28.76 | 87.44 | 85.73 | 121.69 | 132.73 | 178.94 | 207.66 | 287.04 |

### Medium encode, GB/s

| Library | 192 | 256 | 512 | 1024 | 4096 |
|---|---:|---:|---:|---:|---:|
| Turbo-Base64 | 9.5 | 11.6 | 15.6 | 18.9 | 22.6 |
| aklomp-base64 | 5.2 | 8.1 | 15.1 | 16.6 | 23.0 |
| yolow64 | 10.0 | 10.5 | 14.3 | 17.8 | 21.6 |
| encodify | 10.0 | 10.5 | 13.9 | 17.5 | 21.2 |
| base64-turbo | 7.1 | 8.5 | 12.2 | 15.6 | 20.3 |
| tape-base64 | 10.4 | 10.1 | 14.2 | 17.0 | 20.5 |
| fastbase64-fast-avx2 | 7.1 | 9.8 | 15.3 | 16.6 | 19.6 |
| simdutf | 6.4 | 8.6 | 13.3 | 15.2 | 18.6 |
| fastbase64-klomp-avx2 | 6.7 | 9.3 | 13.5 | 13.8 | 15.4 |
| base64-ng | 1.2 | 1.1 | 2.3 | 3.4 | 7.5 |
| base64-simd | 7.5 | 8.2 | 9.1 | 10.3 | 10.8 |
| base64-simd-engine | 5.1 | 6.4 | 8.5 | 8.9 | 9.8 |
| radix64-std | 3.7 | 4.4 | 5.0 | 5.2 | 5.5 |
| simple-base64 | 2.3 | 2.4 | 2.5 | 2.5 | 2.5 |
| base64-scalar | 2.2 | 2.3 | 2.5 | 2.5 | 2.5 |
| data-encoding | 2.3 | 2.3 | 2.4 | 2.5 | 2.5 |
| fastbase64-scalar | 1.9 | 2.0 | 2.0 | 2.0 | 2.1 |
| base64ct | 1.5 | 1.7 | 1.7 | 1.8 | 1.9 |
| fastbase64-chromium | 1.6 | 1.6 | 1.6 | 1.6 | 1.7 |
| fast32-rfc4648 | 1.7 | 1.2 | 1.4 | 1.6 | 1.8 |
| base64_light | 0.6 | 0.6 | 0.7 | 0.8 | 1.0 |

### Large encode, GB/s

| Library | 8192 | 65536 | 1048576 | 4194304 | 16777216 |
|---|---:|---:|---:|---:|---:|
| Turbo-Base64 | 23.1 | 23.8 | 19.4 | 15.0 | 7.3 |
| aklomp-base64 | 24.7 | 23.2 | 20.9 | 19.9 | 7.2 |
| yolow64 | 21.8 | 22.3 | 21.7 | 19.5 | 7.4 |
| encodify | 21.4 | 21.2 | 19.5 | 18.8 | 7.3 |
| base64-turbo | 20.9 | 21.1 | 19.4 | 16.7 | 10.7 |
| tape-base64 | 21.0 | 20.9 | 19.1 | 16.4 | 11.0 |
| fastbase64-fast-avx2 | 20.3 | 19.1 | 17.4 | 16.7 | 7.2 |
| simdutf | 19.4 | 18.9 | 17.1 | 16.6 | 7.3 |
| fastbase64-klomp-avx2 | 15.8 | 15.2 | 14.6 | 12.1 | 7.1 |
| base64-ng | 9.7 | 11.7 | 12.0 | 10.6 | 7.0 |
| base64-simd | 10.9 | 10.8 | 10.5 | 9.2 | 6.7 |
| base64-simd-engine | 10.0 | 10.1 | 9.9 | 9.1 | 6.7 |
| radix64-std | 5.5 | 5.5 | 5.4 | 5.2 | 4.7 |
| simple-base64 | 2.5 | 2.5 | 2.5 | 2.4 | 2.4 |
| base64-scalar | 2.5 | 2.5 | 2.5 | 2.5 | 2.4 |
| data-encoding | 2.5 | 2.5 | 2.5 | 2.4 | 2.4 |
| fastbase64-scalar | 2.1 | 2.1 | 2.0 | 2.0 | 2.0 |
| base64ct | 1.9 | 1.9 | 1.9 | 1.9 | 1.8 |
| fastbase64-chromium | 1.7 | 1.7 | 1.7 | 1.6 | 1.6 |
| fast32-rfc4648 | 1.8 | 1.6 | 1.8 | 1.8 | 1.1 |
| base64_light | 1.0 | 1.1 | 1.1 | 1.1 | 0.7 |

### URL-safe no-padding, GB/s

| Library | 64 | 4096 | 65536 | 1048576 |
|---|---:|---:|---:|---:|
| yolow64 | 4.4 | 21.6 | 22.3 | 21.8 |
| encodify | 4.4 | 21.3 | 21.2 | 19.6 |
| base64-turbo | 3.3 | 20.4 | 21.2 | 19.6 |
| tape-base64 | 3.7 | 21.0 | 21.1 | 19.5 |
| simdutf | 3.3 | 18.7 | 18.8 | 17.0 |
| base64-ng | 0.3 | 7.5 | 11.8 | 12.1 |
| base64-simd | 2.9 | 10.9 | 10.9 | 10.5 |
| radix64-std | 2.7 | 5.5 | 5.5 | 5.4 |
| base64-url | 1.9 | 2.5 | 2.5 | 2.5 |
| base64-scalar | 1.9 | 2.5 | 2.5 | 2.5 |
| data-encoding | 1.9 | 2.5 | 2.5 | 2.5 |
| base64ct | 1.3 | 1.9 | 1.9 | 1.9 |

### Alignment sensitivity, GB/s at 64 KiB

| Library | off 0 | off 1 | off 8 | off 16 | off 32 |
|---|---:|---:|---:|---:|---:|
| Turbo-Base64 | 24.0 | 23.9 | 24.0 | 23.8 | 24.2 |
| base64-turbo | 21.2 | 21.2 | 21.2 | 21.1 | 21.0 |
| encodify | 21.4 | 21.4 | 21.4 | 21.3 | 21.3 |
| simdutf | 18.8 | 18.9 | 18.8 | 19.4 | 18.9 |
| yolow64 | 22.4 | 22.4 | 22.6 | 22.4 | 22.3 |
| base64-simd | 10.8 | 10.8 | 10.7 | 10.8 | 10.8 |

### Relative score (1.0 = best geometric mean per size)

#### short
| Library | score |
|---|---:|
| Turbo-Base64 | 0.955 |
| base64-simd | 0.781 |
| yolow64 | 0.768 |
| tape-base64 | 0.741 |
| base64-turbo | 0.719 |
| encodify | 0.705 |
| fastbase64-klomp-avx2 | 0.675 |
| fastbase64-fast-avx2 | 0.662 |
| simdutf | 0.579 |
| radix64-std | 0.520 |
| simple-base64 | 0.453 |
| aklomp-base64 | 0.440 |
| base64-simd-engine | 0.430 |
| fastbase64-chromium | 0.419 |
| base64-scalar | 0.411 |
| data-encoding | 0.400 |
| fastbase64-scalar | 0.397 |
| base64ct | 0.252 |
| fast32-rfc4648 | 0.203 |
| base64_light | 0.099 |
| base64-ng | 0.067 |

#### medium
| Library | score |
|---|---:|
| Turbo-Base64 | 0.980 |
| yolow64 | 0.933 |
| encodify | 0.919 |
| tape-base64 | 0.911 |
| fastbase64-fast-avx2 | 0.841 |
| aklomp-base64 | 0.782 |
| base64-turbo | 0.776 |
| simdutf | 0.759 |
| fastbase64-klomp-avx2 | 0.737 |
| base64-simd | 0.597 |
| base64-simd-engine | 0.495 |
| radix64-std | 0.310 |
| simple-base64 | 0.160 |
| base64-scalar | 0.157 |
| data-encoding | 0.156 |
| base64-ng | 0.156 |
| fastbase64-scalar | 0.131 |
| base64ct | 0.113 |
| fastbase64-chromium | 0.107 |
| fast32-rfc4648 | 0.100 |
| base64_light | 0.048 |

#### large
| Library | score |
|---|---:|
| aklomp-base64 | 0.906 |
| base64-turbo | 0.886 |
| yolow64 | 0.885 |
| tape-base64 | 0.883 |
| encodify | 0.847 |
| Turbo-Base64 | 0.840 |
| fastbase64-fast-avx2 | 0.782 |
| simdutf | 0.771 |
| fastbase64-klomp-avx2 | 0.639 |
| base64-ng | 0.516 |
| base64-simd | 0.485 |
| base64-simd-engine | 0.465 |
| radix64-std | 0.271 |
| simple-base64 | 0.128 |
| base64-scalar | 0.128 |
| data-encoding | 0.127 |
| fastbase64-scalar | 0.104 |
| base64ct | 0.097 |
| fastbase64-chromium | 0.084 |
| fast32-rfc4648 | 0.081 |
| base64_light | 0.051 |


### Winner by size

| Mode | Size | Library | ns | GB/s |
|---|---:|---|---:|---:|
| short | 8 | Turbo-Base64 | 6.77 | 1.18 |
| short | 12 | Turbo-Base64 | 7.65 | 1.57 |
| short | 16 | Turbo-Base64 | 8.55 | 1.87 |
| short | 24 | tape-base64 | 8.91 | 2.69 |
| short | 32 | Turbo-Base64 | 9.41 | 3.40 |
| short | 48 | tape-base64 | 11.17 | 4.30 |
| short | 64 | Turbo-Base64 | 12.11 | 5.29 |
| short | 96 | tape-base64 | 13.42 | 7.15 |
| short | 128 | Turbo-Base64 | 13.74 | 9.31 |
| medium | 192 | tape-base64 | 18.49 | 10.38 |
| medium | 256 | Turbo-Base64 | 22.00 | 11.64 |
| medium | 512 | Turbo-Base64 | 32.75 | 15.63 |
| medium | 1024 | Turbo-Base64 | 54.04 | 18.95 |
| medium | 4096 | aklomp-base64 | 178.28 | 22.98 |
| large | 8192 | aklomp-base64 | 331.61 | 24.70 |
| large | 65536 | Turbo-Base64 | 2749.59 | 23.83 |
| large | 1048576 | yolow64 | 48212.12 | 21.75 |
| large | 4194304 | aklomp-base64 | 210787.88 | 19.90 |
| large | 16777216 | tape-base64 | 1530616.62 | 10.96 |

## Post-streaming-store update (`yolow64` only)

The 22-implementation table above is the pre-streaming snapshot. After it was
recorded, `yolow64` gained an adaptive streaming-store path: above `3/7` of the
detected last-level cache, the split loop writes the output with 16-byte
non-temporal stores when the output is 16-byte aligned. The threshold is
computed from CPUID at runtime, not hard-coded; on this 16 MiB-L3 host it is
6.86 MiB of input. Details are in [`encode-counters.md`](encode-counters.md).

A targeted three-process run of the same `encode_all` harness with
`--lib yolow64` gives:

| Input | Before (22-way run) | After (campaign A) | Change |
|---:|---:|---:|---:|
| 64 KiB | 22.2 GB/s | 22.5 GB/s | wide prefetch, +1% |
| 1 MiB | 21.8 GB/s | 22.4 GB/s | wide prefetch, +3% |
| 4 MiB | 19.5 GB/s | 21.7 GB/s | wide prefetch, +3% |
| 16 MiB | 7.36 GB/s | **13.0 GB/s** | **1.77x** |

The 16 MiB result is above the recorded external results for `tape-base64`
(10.96 GB/s) and `base64-turbo` (10.70 GB/s). Two three-process campaigns
measured 13.0, 13.1 and 13.3 GB/s; the table uses the conservative 13.0 GB/s. Those external rows were not
rerun. This is a cross-run comparison: external rows are medians from the
22-library process, while the post-update row is an isolated `yolow64` run with
the same harness and buffers. Large-buffer results are insensitive to code
layout; treat the short/medium rows as layout-sensitive.

Short encode also changed after this snapshot. Single-load fast paths for 8, 16
and 32 bytes gave **6.02 ns at 8 B, 6.20 ns at 16 B and 7.29 ns at 32 B** in the
eight-library harness, versus 9.28, 10.27 and 12.98 in the table. In that
harness `yolow64` is now fastest at 8, 12, 16, 24, 32, 48 and 96 B;
Turbo-Base64 wins 64 B and 128 B. In this broader 22-way snapshot,
`tape-base64` is still slightly ahead at 24 B (8.78 versus 9.01). External short
rows were not rerun.

## Observations

- **Medium sizes (192 B–4 KiB).** Turbo-Base64 has the best geometric-mean score (0.980), followed by `yolow64` (0.933), `encodify` (0.919) and `tape-base64` (0.911). The C implementations are not uniformly ahead: aklomp/base64 is weak below 512 B but reaches 23.0 GB/s at 4 KiB.
- **Large sizes (8 KiB–16 MiB).** aklomp/base64 has the best overall score (0.906), narrowly ahead of `base64-turbo` (0.886), `yolow64` (0.885) and `tape-base64` (0.883). aklomp is fastest at 8 KiB and 4 MiB, Turbo-Base64 at 64 KiB, `yolow64` at 1 MiB, and `tape-base64` at 16 MiB. Turbo-Base64 falls to 15.0 GB/s at 4 MiB and 7.3 GB/s at 16 MiB.
- **The 16 MiB regime is different.** In this pre-update snapshot, `tape-base64` (11.0 GB/s) and `base64-turbo` (10.7 GB/s) lead; most SIMD implementations, including `yolow64` (7.4 GB/s), drop to about 7.2–7.4 GB/s. The counter analysis shows this is a DRAM-traffic effect: cached stores move 3.667 bytes per input byte because of read-for-ownership, while streaming stores move 2.333. The post-update `yolow64` result is 13.0–13.3 GB/s at 16 MiB (see above).
- **Short sizes.** Turbo-Base64 wins 8 B, 12 B, 16 B, 32 B, 64 B and 128 B; `tape-base64` wins 24 B, 48 B and 96 B; `yolow64` is in the leading group but not first in this broad harness. See the short-call caveat above.
- **Alignment.** `yolow64` is flat across input offsets 0/1/8/16/32 at 64 KiB (22.3–22.5 GB/s). Turbo-Base64 is also flat (23.7–24.0 GB/s). No implementation shows a large alignment cliff on this input.
- **URL-safe no-padding.** `yolow64` is first at 4 KiB, 64 KiB and 1 MiB (21.7–22.2 GB/s), with `encodify` and `tape-base64` close. At 64 B, `encodify` is marginally faster.
- **Allocation-only APIs.** `base64_light` (1.0 GB/s) and `fast32` (1.6–1.8 GB/s) are far behind caller-buffer implementations because allocation and result construction dominate. They are included because they are real projects, not because they are expected to win a buffer-to-buffer test.
- **Constant-time base64ct** is about 1.8–1.9 GB/s, as expected for a branch/LUT-free scalar design.
- **base64-ng** with the `simd` feature measured about 11.8 GB/s at 64 KiB and 0.3 GB/s at 64 B in this build. We did not investigate whether its runtime backend selection chose a scalar or SIMD path on this host; the row records the measured configuration, not a tuned best case.
- The 4 MiB and 16 MiB rows have noticeable run-to-run variance. They should be read together with the raw CSV, not as exact rankings.

## Reproduction

```sh
# Comparison-only C libraries; all stay outside the yolow64 crate.
git clone --depth 1 https://github.com/powturbo/Turbo-Base64.git /tmp/Turbo-Base64
git clone --depth 1 https://github.com/aklomp/base64.git /tmp/base64-aklomp
git clone --depth 1 https://github.com/lemire/fastbase64.git /tmp/fastbase64-lemire

cargo build --release --manifest-path benchmarks/compare/Cargo.toml --bin encode_all
for i in 1 2 3; do
  taskset -c 4 benchmarks/compare/target/release/encode_all
  cp benchmarks/results/encode_all_summary.csv "/tmp/encode_all_$i.csv"
done
python3 benchmarks/compare/combine_encode.py \
  /tmp/encode_all_1.csv /tmp/encode_all_2.csv /tmp/encode_all_3.csv \
  > benchmarks/results/encode_all_combined.csv

# A single library, useful for isolating code-layout effects:
taskset -c 4 benchmarks/compare/target/release/encode_all --lib yolow64 --out perlib_yolow64
```

`TURBO_BASE64_DIR`, `AKLOMP_BASE64_DIR` and `FASTBASE64_DIR` override the clone locations. The build script compiles aklomp/base64 with AVX2/SSSE3/SSE4/AVX flags and fastbase64 with `SIMD=AVX2` (its Makefile uses `-march=native`). Turbo-Base64 is built by its own makefile. The benchmark binary is not part of the `yolow64` library build; the library remains free of runtime third-party dependencies.

## License note

Turbo-Base64 is GPL-3.0 and is used only in the isolated benchmark binary. aklomp/base64 and fastbase64 are BSD-2-Clause. Rust crates retain their own licenses. `yolow64` itself remains MIT.
