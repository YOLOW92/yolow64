# Encode counters, cache behavior, and crossover points

Measurement date: 2026-11-11. This is the counter-level companion to the
[22-implementation encode comparison](encode-comparison.md). It covers the
`yolow64` encode kernels; external throughput numbers are taken from the
recorded comparison and were not rerun.

Data:

- [`../benchmarks/results/encode_counters.csv`](../benchmarks/results/encode_counters.csv): baseline-subtracted
  instructions, data references, D1/LLd miss rates and branch mispredicts.
- [`../benchmarks/results/encode_counters_raw.csv`](../benchmarks/results/encode_counters_raw.csv): raw
  Cachegrind counts, including the setup baseline.
- [`../benchmarks/results/encode_crossover.csv`](../benchmarks/results/encode_crossover.csv): 1 B–64 KiB
  fine crossover sweep, three process medians.
- [`../benchmarks/results/encode_nt_crossover.csv`](../benchmarks/results/encode_nt_crossover.csv): 4–16 MiB
  cached-store versus streaming-store sweep, three process medians.
- [`../benchmarks/results/encode_prefetch.csv`](../benchmarks/results/encode_prefetch.csv): 8–16 MiB
  prefetch coverage and distance sweep, three process medians.
- [`../benchmarks/results/parallel_encode_combined.csv`](../benchmarks/results/parallel_encode_combined.csv):
  2–16 MiB encode with 1/2/4 workers and the automatic policy, three process
  medians.
- [`../benchmarks/results/encode_cold_combined.csv`](../benchmarks/results/encode_cold_combined.csv):
  three-process medians for warm versus cold-buffer `encode` and
  `encode_streaming`; [`../benchmarks/results/encode_cold_raw.csv`](../benchmarks/results/encode_cold_raw.csv)
  is the raw rounds from the last process.
- [`../benchmarks/results/encode_memory.csv`](../benchmarks/results/encode_memory.csv): peak RSS for
  `encode_to_string` versus `encode_to_writer` at 256 MiB.
- [`../benchmarks/results/8way_short_post.csv`](../benchmarks/results/8way_short_post.csv): post-fast-path
  8–128 B encode medians from the eight-library harness.

## Summary

- Above 4 KiB the split kernel executes **0.568 instructions per input byte**,
  essentially independent of size (4 KiB: 0.635, 64 KiB: 0.572, 16 MiB: 0.568).
- The D1 miss rate of the 8-block loop is **43.75%** by construction: 7 cache-line
  misses per 16 memory references (three input lines plus four output lines per
  192 input bytes). This is not a defect; it is the streaming footprint.
- What changes with size is where those misses are served. LLd miss rate:
  **0.02% at 64 KiB, 1.56% at 1 MiB, 3.12% at 4 MiB, 40.26% at 8 MiB, 43.75%
  at 16 MiB.** The cliff is the 16 MiB L3.
- At 8 MiB and above the kernel is **DRAM-traffic bound**, not instruction bound.
  Cached stores need a read-for-ownership of the output, so they move about
  **3.667 bytes of DRAM traffic per input byte**. Streaming stores move about
  **2.333 bytes per input byte**, a 1.57x traffic reduction.
- On this host, adaptive streaming stores improve 16 MiB encode from
  **7.36 GB/s to 13.0 GB/s (1.77x; other campaigns measured 13.1–13.3 GB/s)**. The crossover is derived from the detected
  last-level cache, not a hard-coded size: `floor = 3/7 * LLC`.
- The remaining large-buffer headroom is small. The 16 MiB streaming result is
  already at roughly 29 GB/s of effective DRAM traffic, near the machine's
  practical memory bandwidth. Further gains need less traffic, and Base64
  expansion is fixed at 4/3.
- One-shot cold encoding is about 3x slower than the warm repeated-call table:
  5.7–8.0 GB/s from 4 KiB to 4 MiB. `Codec::encode_streaming` uses NT stores
  from 4 KiB and recovers 7.9–13.4 GB/s (1.39–1.68x). `Codec::encode` stays the
  cache-friendly default for reused or immediately reread buffers.

## Why TSC and Cachegrind

Hardware performance counters are unavailable: `/proc/sys/kernel/perf_event_paranoid`
is `4`, so `perf stat -e cycles,instructions` is denied. Two independent
measurements were used instead:

1. **TSC (`rdtsc`) timing.** The TSC is a constant reference clock on this host
   at roughly 2.9 GHz. `tsc_cycles_per_byte` is therefore a normalized time
   metric, not core cycles. It does not include frequency scaling. It is still
   useful for comparing variants because both sides are normalized the same way.
2. **Cachegrind 3.26** for instruction counts, simulated cache misses and branch
   prediction. Cachegrind was extracted from the Ubuntu package into `/tmp` and
   kept outside the repository. Simulated geometry: I1 and D1 = 32 KiB, 8-way,
   64 B lines; LL = 16 MiB, 16-way, 64 B lines. This matches the host's 32 KiB
   L1d per core and 16 MiB L3. Cachegrind models L1 and LL, not the 256 KiB L2.

Cachegrind does not model non-temporal stores, so all streaming-store numbers
come from TSC timing, not from Cachegrind. The counter tables describe the
cached split kernel.

## Method

### Counter harness

Two ignored tests in `src/threshold_tests.rs`:

- `encode_profile_one` allocates an input and output buffer, fills the input,
  sets `PROFILE_VARIANT`, and calls `STANDARD.encode` `PROFILE_ITERS` times.
- `encode_profile_setup` performs exactly the same allocation, input fill and
  empty loop, but no encode calls.

For every size and iteration count, the setup counters are subtracted from the
encode counters:

```text
net instructions = encode I refs - setup I refs
instructions/call = net instructions / iters
instructions/byte = instructions/call / input size
D1 miss rate = (encode D1 misses - setup D1 misses) / (encode D refs - setup D refs)
```

This removes the allocation, the input fill and the test loop, which are large
enough to distort the per-call numbers at 8–16 MiB. Without subtraction, the
16 MiB instruction count is inflated by about 25%.

### Variants

| Name | Kernel | Used by production |
|---|---|---|
| `auto` | dispatch: `load16` <= 96 B, direct 97–4095 B, split >= 4096 B, streaming split >= 3/7 LLC | yes |
| `direct` | `encode`, 32-byte-load kernel | 97–4095 B |
| `load16` | `encode_load16`, two 16-byte loads | 24–96 B |
| `split` | `encode_unroll8_split::<0>`, 8-block loop with cached stores | 4 KiB–stream floor |
| `split_nt16` | `encode_unroll8_split::<2>`, two 16-byte `movntdq` per 32-byte vector | >= stream floor |
| `split_nt32` | `encode_unroll8_split::<1>`, 32-byte `vmovntdq` | experiment only |

The production streaming variant is `split_nt16`. It needs only 16-byte output
alignment, which large allocator blocks normally provide. If the output is not
16-byte aligned, production falls back to cached `split`.

### Timing harness

- Fine sweep: 1 B–64 KiB, 9 rounds per process, 3 processes, variant order
  rotated per round. Input and output are 64-byte aligned.
- Large sweep: 4–16 MiB, 8 rounds per process, 3 processes, 5 variants rotated
  per round. Each sample encodes about 64 MiB of input.
- Short sizes are noisy below about 32 B. Treat differences under ~1 ns as
  layout/noise, not a stable ranking. The dedicated eight-library harness gives
  12 B ≈ 7.3 ns; this isolated lab gives ≈ 10 ns because of code layout.

## Instruction and cache counters

Baseline-subtracted `auto` rows. `LLd misses/call` is the number of simulated
last-level cache line misses per encode call.

| Input | Instructions/call | Instructions/byte | D1 miss | LLd miss | LLd misses/call | Branch mispredict |
|---:|---:|---:|---:|---:|---:|---:|
| 8 B | 182.0 | 22.749 | ≈0 | ≈0 | 0 | ≈0 |
| 12 B | 157.0 | 13.083 | ≈0 | ≈0 | 0 | ≈0 |
| 16 B | 218.0 | 13.624 | ≈0 | ≈0 | 0 | ≈0 |
| 24 B | 181.0 | 7.541 | ≈0 | ≈0 | 0 | ≈0 |
| 64 B | 261.0 | 4.078 | ≈0 | ≈0 | 0 | ≈0 |
| 128 B | 311.0 | 2.429 | ≈0 | ≈0 | 0 | ≈0 |
| 256 B | 398.9 | 1.558 | ≈0 | ≈0 | 0 | ≈0 |
| 1 KiB | 870.9 | 0.851 | ≈0 | ≈0 | 0 | 2.11% |
| 4 KiB | 2,599.8 | 0.635 | ≈0 | ≈0 | 0 | 1.98% |
| 64 KiB | 37,463.6 | 0.572 | 43.34% | 0.02% | 1.3 | 0.57% |
| 1 MiB | 595,477.2 | 0.568 | 43.73% | 1.56% | 1,366 | 0.13% |
| 2 MiB | 1,190,723.1 | 0.568 | 43.74% | 3.12% | 5,462 | 0.11% |
| 4 MiB | 2,381,291.8 | 0.568 | 43.75% | 3.12% | 10,926 | 0.08% |
| 8 MiB | 4,762,294.5 | 0.568 | 43.75% | 40.26% | 281,472 | 0.06% |
| 16 MiB | 9,524,466.5 | 0.568 | 43.75% | 43.75% | 611,693 | 0.04% |

The 1–4 KiB branch-mispredict percentage looks larger only because those calls
execute far fewer branches. In absolute terms it is about one mispredict per
call. At 64 KiB and above it is 0.04–0.57%, which is not material.

### Why 43.75% D1 misses is structural

The split loop consumes 192 input bytes per iteration of `group4!()` twice:

- 8 × 32-byte input loads = 3 cache lines
- 8 × 32-byte output stores = 4 cache lines
- 16 memory references, 7 line misses

`7 / 16 = 43.75%`. The measured 43.75% at 8–16 MiB confirms that Cachegrind is
seeing the intended pattern. At 64 KiB it is 43.34% because the prefix and tail
add some cached references.

The D1 rate is therefore not the metric that changes with size. The LLd rate is:
the same 7 misses are absorbed by L2/L3 when the working set fits, and go to
DRAM when it does not.

### Kernel selection at short/medium sizes

Instructions per call for the tested kernels:

| Input | `auto` | `direct` | `load16` | `split` | Best |
|---:|---:|---:|---:|---:|---|
| 24 B | 181.0 | 199.0 | 183.0 | 212.0 | `load16` |
| 64 B | 261.0 | 274.0 | 263.0 | 324.0 | `load16` |
| 128 B | 311.0 | 311.0 | 316.0 | 411.0 | `direct` |
| 1 KiB | 870.9 | 870.7 | 878.9 | 870.9 | `direct`/`split` tie |
| 4 KiB | 2,599.8 | 2,758.8 | 2,830.9 | 2,598.9 | `split` |

This supports the current thresholds:

- **12–23 B:** one 12-byte SSE chunk plus scalar tail.
- **24–96 B:** two 16-byte loads (`load16`).
- **97–4,095 B:** 32-byte-load direct kernel.
- **4,096 B and above:** 8-block split loop.
- **At or above `3/7 × LLC`:** the same split loop with 16-byte streaming stores.

At 4,096 B `split` and `direct` are effectively tied (2,598.9 versus 2,758.8
instructions; 195.70 versus 195.81 ns), so 4,096 is a conservative threshold.
The split loop pulls ahead clearly by 64 KiB.

## Fine crossover sweep

Three-process medians. The best variant at each size is shown. `TSC c/B` is
reference-clock cycles per input byte; multiply by 2.9 to compare with GHz
(0.130 TSC c/B ≈ 22.3 GB/s).

| Input | Best variant | ns/call | TSC c/B | Note |
|---:|---|---:|---:|---|
| 8 B | `auto` | 9.79 | 3.552 | dispatch/latency dominated |
| 12 B | tie | 10.03 | 2.428 | 12-byte SSE chunk |
| 16 B | tie | 12.87 | 2.336 | 12-byte chunk + 4-byte scalar tail |
| 24 B | `auto` | 10.73 | 1.297 | 16-byte-load kernel |
| 32 B | `auto` | 14.69 | 1.333 | 16-byte-load kernel + tail |
| 48 B | `auto` | 11.83 | 0.715 | 16-byte-load kernel |
| 64 B | `auto` | 15.21 | 0.690 | 16-byte-load kernel |
| 96 B | `auto` | 13.48 | 0.407 | last `load16` size |
| 97 B | `load16` | 15.27 | 0.457 | `direct` is 16.64 ns, tie-ish |
| 128 B | `direct` | 19.06 | 0.432 | `load16` is 19.19 ns |
| 192 B | `load16` | 18.54 | 0.280 | `direct` is 19.35 ns, tie-ish |
| 256 B | `direct` | 24.59 | 0.279 | |
| 512 B | `direct` | 35.75 | 0.203 | |
| 1 KiB | `direct` | 58.98 | 0.167 | |
| 4 KiB | `split` | 195.70 | 0.139 | `direct` is 195.81 ns |
| 4.5 KiB | `split` | 211.91 | 0.133 | split starts to win |
| 5 KiB | `direct` | 236.97 | 0.133 | within noise of split |
| 64 KiB | `split` | 2,929.05 | 0.130 | |

Two non-monotonic spots are real rather than noise:

- **16 B is slower than 24 B.** The 12–23 B path writes a 12-byte SSE chunk and
  then handles the remaining 4 bytes with a scalar tail. At 16 B that tail is
  relatively expensive. A dedicated 16-byte single-load path is the clearest
  remaining short-input opportunity.
- **32 B is slower than 48 B.** Both use the 16-byte-load kernel, but 32 B has
  a larger tail fraction.

The 96/97 and 4,096/4,608 boundaries are not sharp. The differences around them
are at or below the run-to-run noise, so the current thresholds are chosen to
keep the hot path simple rather than to chase a sub-nanosecond crossover.

The 8/16/32 B rows above predate the single-load fast paths. The stable
eight-library harness, after the fast paths, gives:

| Input | Before | After | Change |
|---:|---:|---:|---:|
| 8 B | 9.28 | **6.02** | -35% |
| 12 B | 7.33 | 7.35 | unchanged |
| 16 B | 10.27 | **6.20** | -40% |
| 24 B | 9.25 | 9.01 | -3% |
| 32 B | 12.98 | **7.29** | -44% |
| 48 B | 10.40 | 9.95 | -4% |
| 64 B | 13.62 | 13.41 | -1% |
| 96 B | 12.36 | 11.43 | -8% |
| 128 B | 17.51 | 17.40 | unchanged |

The fast paths load 8/16/32 bytes once, run the same 24-byte conversion, and
overwrite the final padding characters. In the eight-library harness `yolow64`
is now fastest at 8, 12, 16, 24, 32, 48 and 96 B. The broader 22-way snapshot
still has `tape-base64` slightly ahead at 24 B (8.78 versus 9.01). The remaining
eight-library gaps are 64 B (13.41 versus Turbo-Base64 12.11) and 128 B (17.40
versus Turbo 13.74).

## Large-buffer streaming crossover

The working set of an encode is the input plus the output: `N + 4N/3 = 7N/3`.
For an LLC of `C` bytes, it stops fitting when `N >= 3C/7`. On this host
`C = 16 MiB`, so the crossover is **6.857 MiB** of input.

Three-process medians, GB/s. `cached` is `split`; `NT16` is two 16-byte
streaming stores per vector; `NT32` is one 32-byte streaming store. The
production `auto` path uses `cached` below 6.857 MiB and `NT16` above it.

| Input | Cached `split` | `NT16` | `NT32` |
|---:|---:|---:|---:|
| 4.00 MiB | **21.05** | 15.60 | 18.06 |
| 4.50 MiB | 17.71 | 15.62 | **19.03** |
| 5.00 MiB | 14.40 | 16.57 | **18.18** |
| 5.50 MiB | 13.04 | 14.85 | **20.23** |
| 6.00 MiB | 12.11 | 14.55 | **18.53** |
| 6.50 MiB | 14.67 | **16.42** | 15.26 |
| 7.00 MiB | 10.43 | 14.54 | **15.80** |
| 7.50 MiB | 10.07 | 14.53 | **16.04** |
| 8.00 MiB | 9.57 | 15.15 | **15.35** |
| 12.00 MiB | 7.68 | **13.27** | 12.98 |
| 16.00 MiB | 7.18 | **12.50** | 11.13 |

Below 6.5 MiB the result is sensitive to what the previous variant left in the
L3, so the 4–6 MiB cells are noisier than the table suggests. The robust part is
7 MiB upward: cached stores fall to 10.4, 10.1, 9.6, 7.7 and 7.2 GB/s, while
`NT16` stays at 14.5, 14.5, 15.2, 13.3 and 12.5 GB/s. The 16 MiB improvement is
**1.74x** for NT16 alone in this isolated sweep; the production path with
three-line prefetch reaches 13.0 GB/s (1.77x over the 22-way 7.36).

`NT16` and `NT32` are close. `NT32` is slightly better at 8 MiB, `NT16` is
better at 12–16 MiB, and `NT16` needs only 16-byte output alignment. Production
therefore uses `NT16`.

### Prefetch coverage

The split loop consumes 192 input bytes per iteration: three 64-byte cache
lines. The original loop issued **one** prefetch per iteration and relied on the
hardware prefetcher for the other two lines. The production loop now issues
three prefetches at offsets 0, 64 and 128 from the same look-ahead address.

Focused sweep, three-process medians, GB/s:

| Input | 1 line, 8 KiB | 3 lines, 8 KiB | Best 3-line distance |
|---:|---:|---:|---:|
| 8 MiB | 14.46 | 15.85 | 16.38 (16 KiB) |
| 10 MiB | 14.34 | **15.88** | 15.88 (8 KiB) |
| 12 MiB | 13.34 | **14.88** | 14.88 (8 KiB) |
| 14 MiB | 12.73 | 13.72 | 14.13 (12 KiB) |
| 16 MiB | 12.95 | 13.15 | 13.30 (32 KiB) |

Three-line coverage is the robust part: it is **+10%** at 8 MiB and
**+10–14%** at 10–14 MiB, with +2% at 16 MiB. Distance tuning beyond 8 KiB is
within run noise, so production keeps the 8192-byte look-ahead. No prefetch is
clearly worse (12.3 GB/s at 8 MiB, 10.3 at 16 MiB), and `prefetchnta` is not
consistently better than `prefetcht0`.

The `encode_all` campaign confirms the combined effect: 16 MiB went from
12.1–12.4 GB/s with NT alone to **13.0–13.3 GB/s** with NT plus three-line
prefetch, and 1 MiB / 4 MiB gained about 3%. The 64 KiB row is unchanged within
noise. The `NT16` column in the table above is the old one-line measurement; the
production path is the three-line version.

## Traffic model and remaining headroom

### DRAM bytes per input byte

Per 192 input bytes, the split loop touches 3 input lines and 4 output lines:

- **Cached stores:** read the 3 input lines, read the 4 output lines for
  read-for-ownership, then write the 4 output lines back:
  `(3 + 4 + 4) × 64 = 704 B` per 192 B input = **3.667 B/B**.
- **Streaming stores:** read the 3 input lines and write the 4 output lines once:
  `(3 + 4) × 64 = 448 B` per 192 B input = **2.333 B/B**.

The traffic ratio is `3.667 / 2.333 = 1.571`. The measured 16 MiB speedup is
1.64–1.77x across campaigns, better than the traffic ratio because streaming stores also
avoid cache pollution and the latency of the ownership read.

### 16 MiB

At 7.36 GB/s input, cached stores move `7.36 × 3.667 = 27.0 GB/s` of DRAM
traffic. At 13.0–13.3 GB/s input, streaming stores move `30.3–31.0 GB/s`. Both
are at the machine's practical memory bandwidth. The gain is a traffic
reduction, not a better instruction sequence.

The theoretical streaming ceiling at 27 GB/s is `27 / 2.333 = 11.6 GB/s` input;
the measured 13.0 GB/s implies about 30 GB/s effective bandwidth. Further
large-buffer improvement would require either more memory bandwidth or fewer
bytes moved. Base64 necessarily writes 4/3 of the input, so 2.333 B/B is the
floor for this algorithm on AVX2. AVX-512 is excluded, and 32-byte streaming
stores are not better than two 16-byte stores here.

### 8 MiB

At 8 MiB the working set is 18.7 MiB, just over the 16 MiB L3. LLd is 40.26%,
not 100%, so a fraction of the traffic is still served by L3. Cached stores
reach 9.57 GB/s (≈35 GB/s of combined traffic), streaming stores 15.15 GB/s
(≈35 GB/s). Both are limited by the mixed L3/DRAM path. The streaming gain is
still 1.58x.

### 4 MiB and below

The working set is 9.3 MiB at 4 MiB, which fits in the 16 MiB L3. LLd is only
3.12%, and cached stores win: 21.05 GB/s versus 15.60 GB/s for `NT16` in the
same sweep. Forcing streaming stores here would move output to DRAM while the
input is still L3-resident, which is the wrong trade. This is why the adaptive
floor is 6.857 MiB, not a fixed "large" threshold like 4 MiB.

At 64 KiB–1 MiB the kernel is L2/L3-bandwidth bound. 64 KiB reaches 22.4 GB/s
with 0.572 instructions/byte; 1 MiB reaches 21.8 GB/s with 0.568. The D1 miss
rate is still 43.75%, but LLd is only 0.02–1.56%, so those misses are absorbed
by L2/L3. The instruction count is already low; the remaining limit is the
load/store path and cache bandwidth.

### Short inputs

At 12 B the encode is 157 instructions and 13.08 instructions/byte. At 8 B the
old path was 182 instructions and 22.75 instructions/byte. These are
latency-dominated, not bandwidth-dominated. The 8/16/32-byte single-load paths
now reuse the 24-byte conversion and overwrite the final padding characters. In
the stable eight-library harness, 8 B drops from 9.28 to 6.02 ns, 16 B from
10.27 to 6.20 ns, and 32 B from 12.98 to 7.29 ns. The earlier 16 B anomaly
(scalar tail slower than 24 B) is closed. The remaining short gaps are 64 B and
128 B, where Turbo-Base64 still leads.

## Parallel encode scheduling

The single-thread streaming path changes the parallel crossover. The old policy
used at most two workers and disabled parallel once `input + output` exceeded
7/8 of the LLC. Measured with `encode_parallel_with_threads` and rotated rounds:

| Input | 1 thread | 2 threads | 4 threads | Automatic policy |
|---:|---:|---:|---:|---:|
| 4 MiB | 18.1 | 19.5 | **22.5** | 22.1 |
| 5 MiB | 16.8 | 18.1 | **21.1** | 20.6 |
| 6 MiB | 14.0 | 15.6 | **17.2** | 17.0 |
| 6.5 MiB | 12.5 | — | **15.1** | 14.7 |
| 7 MiB | **15.4** | — | 13.4 | 14.2 |
| 8 MiB | **15.0** | 11.0 | 10.9 | 14.3 |
| 12 MiB | **13.6** | 7.8 | 8.3 | 13.3 |
| 16 MiB | **12.4** | 11.5 | 7.8 | 11.9 |

The absolute 1-thread 4 MiB cell is lower than the 21.2 GB/s in the isolated
`encode_all` run because this lab rotates thread counts and keeps thread-pool
state warm. The same-process ratios are the useful part.

The measurements give a clean capacity-based policy:

- **3 MiB and below:** single-thread. Thread creation costs more than the
  cache-resident gain (3 MiB: 1 thread 19.6, auto 19.8, 4 threads 16.9). The
  automatic API returns directly to `encode` below 4 MiB, avoiding the wrapper
  syscall and a second output-length check.
- **4–6.5 MiB:** four workers. The whole working set fits in the 16 MiB L3, and
  four workers use its bandwidth: +24% at 4 MiB, +23% at 6 MiB, +21% at 6.5 MiB.
- **7 MiB and above:** one worker with streaming stores. Four cached workers
  thrash the LLC, while the single-thread NT path is already near the DRAM
  limit: 15.4 versus 13.4 GB/s at 7 MiB, 15.0 versus 10.9 at 8 MiB, 12.4 versus
  7.8 at 16 MiB.

The production policy is therefore:

```text
MIN_PARALLEL_ENCODE = 4 MiB
MAX_ENCODE_WORKERS = 4
parallel while input + output <= detected LLC
single-thread streaming path above the LLC
```

`available_parallelism` is cached in a `OnceLock`; it can issue a syscall, and
the wrapper cost was visible on 2–3 MiB calls before caching.

The encode chunk boundaries are rounded down to a multiple of four groups.
Four groups produce 16 output bytes, so every worker after the first starts on
a 16-byte boundary and remains eligible for the streaming-store path. Without
this rounding, the second worker of a 16 MiB / 2-thread split can start at an
8-byte offset and silently fall back to cached stores.

Decode is intentionally unchanged in this round: it still uses two workers and
a 7/8-LLC margin. Its own crossover will be measured after the encode work.

## Cold one-shot behavior

All rows above reuse one input/output pair: they measure warm repeated calls.
A real one-shot encode usually reads a cold input and writes an output that the
caller then sends, writes or compresses. `encode_cold` rotates through a 64 MiB
working set so every call sees cold buffers.

Three-process medians, GB/s:

| Input | Warm `encode` | Cold `encode` | Cold `encode_streaming` | Cold speedup |
|---:|---:|---:|---:|---:|
| 4 KiB | 20.12 | 5.65 | 7.88 | 1.39x |
| 16 KiB | 22.07 | 6.79 | 10.04 | 1.48x |
| 64 KiB | 22.44 | 7.44 | 11.68 | 1.57x |
| 256 KiB | 21.97 | 7.31 | 11.46 | 1.57x |
| 1 MiB | 22.39 | 7.83 | 12.67 | 1.62x |
| 4 MiB | 21.77 | 7.99 | 13.41 | 1.68x |
| 16 MiB | 13.04 | 11.72 | 11.68 | 1.00x |

The warm table overstates the cache-resident case by about 3x. Once the input
is cold, the cached-store path pays for the output read-for-ownership and
lands at 5.7–8.0 GB/s. Streaming stores remove that read: `encode_streaming`
reaches 7.9–13.4 GB/s from 4 KiB to 4 MiB, a 1.39–1.68x improvement.

The default stays split on purpose:

- `Codec::encode` keeps the output cacheable below `3/7 LLC`. This is best when
  the same output buffer is reused or the result is immediately reread from L3.
- `Codec::encode_streaming` uses 16-byte streaming stores from 4 KiB upward
  when the output is 16-byte aligned. It is the one-shot API for sockets, files
  and compressors.
- At 16 MiB both paths already use streaming stores, so the two APIs converge.

The 4 KiB threshold is measured: at 1 KiB, NT is 3.65 GB/s versus 3.63 for the
cached direct kernel; at 4 KiB, NT is 7.34 versus 5.85. At 256 B, NT is worse
(1.61 versus 2.80). Streaming only pays once the bulk loop has enough full
groups to amortize the prefix and tail.

### Peak memory

`encode_to_writer` processes 48 KiB multiples of three through a fixed 64 KiB
output buffer. For a 256 MiB input, `/usr/bin/time -v` reports:

| Path | Peak RSS |
|---|---:|
| `encode_to_string` | 613,640 KiB |
| `encode_to_writer` | 264,256 KiB |

That is a 349,384 KiB (about 341 MiB) reduction: the writer path holds the input plus one 64 KiB
buffer instead of the full 341 MiB encoded output. The cost is an extra copy
through the buffer and one `write_all` per 64 KiB. Wrap a syscall-heavy writer
in `BufWriter`; for in-memory output where peak memory is acceptable,
`encode_to_string` avoids the copy.

## Adaptive threshold

### What is detected

`stream_floor()` is computed once per process with `OnceLock`:

1. Read CPUID leaf 4 (Intel deterministic cache parameters). If that leaf is
   supported, parse every entry and keep the largest data/unified cache level.
2. Otherwise read AMD leaf `0x8000_001d`, which uses the same layout.
3. Otherwise read AMD leaf `0x8000_0006`: L3 size in bits 31:18 in 512 KiB
   units, or L2 size in bits 31:16 in 256 KiB units.
4. Accept only sizes in **64 KiB–2 GiB**. An implausible or missing value means
   "unknown LLC", and streaming stores are disabled.
5. `stream_floor = max(3/7 × LLC, 64 KiB)`.

The 3/7 factor is the working-set boundary derived above. It is not a tuned
constant for this i7-10700; it follows from the 4/3 output expansion.

### What it becomes on other processors

| LLC | Input floor `3/7 × LLC` | Input + output at the floor |
|---:|---:|---:|
| 512 KiB (L2-only) | 219 KiB | 512 KiB |
| 1 MiB | 439 KiB | 1 MiB |
| 4 MiB | 1.71 MiB | 4 MiB |
| 8 MiB | 3.43 MiB | 8 MiB |
| **16 MiB (this host)** | **6.86 MiB** | **16 MiB** |
| 32 MiB | 13.71 MiB | 32 MiB |
| 64 MiB | 27.43 MiB | 64 MiB |

On a CPU with a larger LLC the streaming path switches on later; on a smaller
LLC it switches on earlier. On a CPU with no L3 and a 512 KiB L2, the floor is
219 KiB, so a 256 KiB encode uses streaming stores while a 128 KiB encode stays
cached.

### Alignment and fallback

Streaming stores require alignment. The production path checks:

```text
input.len() >= stream_floor() && output.as_mut_ptr() as usize % 16 == 0
```

If either condition fails, it uses the cached split loop. This covers allocators
that return only 8- or 16-byte alignment, and callers that pass a sub-slice with
an unaligned start. `NT16` was chosen over `NT32` so that 16-byte alignment is
sufficient; `NT32` would require 32-byte alignment and was not faster overall.

### Testing without other hardware

The CPUID parser and the threshold math are unit-tested with synthetic register
values, so the logic does not depend on having a second physical machine:

- `cache_tests::parses_intel_and_amd_l3_shapes`: 16 MiB and 32 MiB L3 leaves.
- `cache_tests::rejects_instruction_and_null_entries`: type 0 and type 2.
- `cache_tests::rejects_implausible_sizes`: too small and too large.
- `cache_tests::floor_is_three_sevenths_of_llc`: 16 MiB, 512 KiB, 64 KiB and
  unknown.
- `cache_tests::detected_cache_is_sane`: prints the actual detected LLC and
  floor on the running machine.
- `streaming_encode_matches_reference`: exercises the production `auto` path at
  `floor + 4103` bytes with both 64-byte-aligned and unaligned output, and
  compares the bytes with the `base64` crate.

On this host the diagnostic test prints:

```text
llc_bytes=Some(16777216) stream_floor=7190235
```

On a different machine, run:

```sh
cargo test --release --lib detected_cache_is_sane -- --nocapture
cargo test --release --lib streaming_encode_matches_reference
```

The first prints the detected values; the second checks correctness of the
selected path. The large-buffer timing sweep is `encode_nt_lab`, described below.

## Post-change targeted result

The 22-way table in [`encode-comparison.md`](encode-comparison.md) is the
pre-streaming snapshot. After the adaptive path was added, `encode_all --lib
yolow64` was run in two three-process campaigns with the same harness and
buffers. The table uses campaign A; campaign B measured 22.5 GB/s at 64 KiB,
22.4 GB/s at 1 MiB, 21.8 GB/s at 4 MiB and 13.0 GB/s at 16 MiB.

| Input | Before (22-way run) | After (conservative) | Change |
|---:|---:|---:|---:|
| 64 KiB | 22.2 GB/s | 22.5 GB/s | wide prefetch, +1% |
| 1 MiB | 21.8 GB/s | 22.4 GB/s | wide prefetch, +3% |
| 4 MiB | 19.5 GB/s | 21.7 GB/s | wide prefetch, +3% |
| 16 MiB | 7.36 GB/s | **13.0 GB/s** | **1.77x** |

The 16 MiB result is above the recorded external results for `tape-base64`
(10.96 GB/s) and `base64-turbo` (10.70 GB/s) in the 22-way table. Those external
rows were **not** rerun; the comparison is cross-run and should be read with the
caveats in the comparison report.

An eight-library validation run (only the `yolow64` rows were used) confirmed
there is no short-input regression after the prefetch change: 8–128 B stayed
within ±2%, 1 MiB improved by 3%, and 4 MiB improved by 20%. The external rows
from that run were not re-ranked; the 22-way table above remains the recorded
snapshot.

The wide-prefetch campaigns are kept as
[`../benchmarks/results/perlib_yolow64_wide_{1,2,3}.summary.csv`](../benchmarks/results/perlib_yolow64_wide_1.summary.csv),
[`../benchmarks/results/perlib_yolow64_wide_b_{1,2,3}.summary.csv`](../benchmarks/results/perlib_yolow64_wide_b_1.summary.csv)
and
[`../benchmarks/results/perlib_yolow64_final_{1,2,3}.summary.csv`](../benchmarks/results/perlib_yolow64_final_1.summary.csv).
Each `.summary.csv` has a matching `.raw.csv` with all rounds. The table uses the
median of the wide campaigns; the final campaign measured 13.3 GB/s at 16 MiB.

## Reproduction

```sh
# TSC crossover, 1 B–64 KiB (three processes recommended)
cargo test --release --lib encode_crossover_lab -- --ignored --nocapture

# TSC large-buffer sweep, 4–16 MiB
cargo test --release --lib encode_nt_lab -- --ignored --nocapture

# Warm versus cold-buffer encode and encode_streaming
cargo build --release --manifest-path benchmarks/compare/Cargo.toml --bin encode_cold
taskset -c 4 benchmarks/compare/target/release/encode_cold

# Peak RSS for owned versus writer encode (256 MiB input)
cargo build --release --manifest-path benchmarks/compare/Cargo.toml --bin encode_memory
/usr/bin/time -f 'max_rss_kb=%M' \
  benchmarks/compare/target/release/encode_memory string 256
/usr/bin/time -f 'max_rss_kb=%M' \
  benchmarks/compare/target/release/encode_memory writer 256

# Cachegrind counters for one configuration
PROFILE_SIZE=1048576 PROFILE_VARIANT=0 PROFILE_ITERS=16 \
VALGRIND_LIB=/tmp/valgrind_pkg/root/usr/libexec/valgrind \
/tmp/valgrind_pkg/root/usr/bin/valgrind \
  --tool=cachegrind --cache-sim=yes --branch-sim=yes \
  --I1=32768,8,64 --D1=32768,8,64 --LL=16777216,16,64 \
  --cachegrind-out-file=/tmp/cg.out \
  target/release/deps/yolow64-<hash> encode_profile_one --ignored --nocapture

# Setup baseline with the same size and iteration count
PROFILE_SIZE=1048576 PROFILE_ITERS=16 \
VALGRIND_LIB=/tmp/valgrind_pkg/root/usr/libexec/valgrind \
/tmp/valgrind_pkg/root/usr/bin/valgrind \
  --tool=cachegrind --cache-sim=yes --branch-sim=yes \
  --I1=32768,8,64 --D1=32768,8,64 --LL=16777216,16,64 \
  --cachegrind-out-file=/tmp/cg-setup.out \
  target/release/deps/yolow64-<hash> encode_profile_setup --ignored --nocapture
```

`cargo test --release --lib --no-run` prints the test binary hash. The raw CSV
was produced by running the two commands above for each size/variant and
subtracting the setup rows.

## Caveats

- TSC is a constant reference clock, not core cycles. `tsc_cycles_per_byte` is
  normalized time; it does not expose frequency scaling or IPC directly.
- Cachegrind simulates a generic cache and does not model non-temporal stores,
  hardware prefetchers, DRAM latency or bandwidth. Use it for instruction counts
  and miss rates, not for absolute time.
- The machine is shared and `schedutil` is active. The 4–8 MiB region is
  sensitive to cache state left by the previous variant. The 8 MiB+ medians are
  stable; the 4–6.5 MiB cells are not.
- The 22-way external results are a recorded snapshot. They were not rerun when
  the streaming path was added. Only `yolow64` rows were remeasured.
- Counter runs use the test build, which contains the `ENCODE_VARIANT` hook.
  Production builds compile that hook out, so short-call instruction counts may
  be a few instructions lower; at 4 KiB and above the difference is below
  0.01%. The test build also defaults to the narrow one-line prefetch, while
  production issues three-line prefetch; the extra two prefetch instructions
  add under 1% to the instruction count and do not change the cache-miss
  structure.
- The adaptive threshold assumes the output is not immediately reread. If an
  application encodes a >= 6.86 MiB buffer and then immediately reads the output
  from cache, cached stores could be preferable. There is no public switch for
  that case yet; the default favors streaming for large outputs.
- On a multi-CCD AMD CPU the detected L3 is the local CCD's L3, which is the
  right unit for a single-threaded encode. On a hybrid CPU with per-core cache
  differences, the `OnceLock` keeps the first detected value; this is a minor
  limitation, not a correctness issue.
