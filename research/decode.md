# Decode status and cold-buffer measurements

Measurement date: 2026-11-11. This report covers the decode round after the
encode and memory work. No decode regression was shipped: every experimental
kernel below was measured, and the ones that did not win were reverted.

Data:

- [`../benchmarks/results/decode_cold_combined.csv`](../benchmarks/results/decode_cold_combined.csv):
  three-process medians for warm versus cold-buffer decode.
- [`../benchmarks/results/8way_combined.csv`](../benchmarks/results/8way_combined.csv): recorded warm
  eight-library decode table (the pre-decode-round snapshot).
- [`../benchmarks/results/8way_decode.csv`](../benchmarks/results/8way_decode.csv): six-process
  `yolow64` large-input decode medians for the final loop.
- [`../benchmarks/results/8way_short_decode.csv`](../benchmarks/results/8way_short_decode.csv):
  six-process `yolow64` short decode medians after the exact-length fast path.
- [`../benchmarks/results/8way_decode_all.csv`](../benchmarks/results/8way_decode_all.csv)
  and [`../benchmarks/results/8way_short_all.csv`](../benchmarks/results/8way_short_all.csv):
  six-process medians for every library in the eight-way decode run, used by
  the public heatmaps.
- [`../benchmarks/results/decode_fair.csv`](../benchmarks/results/decode_fair.csv): full-check comparison
  of `yolow64`, Turbo-Base64 with `FULLCHECK=1`, and Turbo-Base64's default
  partial-check build (earlier snapshot).
- [`../benchmarks/results/turbo_check_verify.csv`](../benchmarks/results/turbo_check_verify.csv): verified
  rebuild of both Turbo-Base64 configurations, direct kernel and dispatch path.
- [`turbo-check.md`](turbo-check.md): build-flag, dispatch-target and
  disassembly verification for the 25%/100% check coverage.

## Warm decode

Eight-library medians, GB/s (higher is better):

| Input | yolow64 (before) | yolow64 (shipped) | Turbo full-check | Turbo default (partial) |
|---:|---:|---:|---:|---:|
| 8 KiB | 18.31 | **19.64** | 20.06 | 26.34 |
| 16 KiB | 18.53 | **19.54** | 19.68 | 26.41 |
| 64 KiB | 18.76 | **20.08** | 19.36 | 26.90 |
| 256 KiB | 19.08 | **20.16** | 16.76 | 20.80 |
| 1 MiB | 19.18 | **20.17** | 16.68 | 20.57 |
| 4 MiB | 16.62 | 17.06 | 13.89 | 17.01 |

The Turbo columns are the 2026-11-11 verification rebuild
([`turbo-check.md`](turbo-check.md)). The earlier snapshot was 20.19 / 19.70 /
19.71 / 16.95 / 16.74 / 14.00 for the full build and 26.87 / 26.68 / 26.03 /
20.86 / 20.69 / 16.81 for the default build.

The default Turbo build enables `CHECK0` but leaves `CHECK1` empty. In the
256-byte DS128 hot loop `CHECK0` runs on the first of four 32-byte blocks and
`CHECK1` on the next three, so the default build checks **one block in four
(25%)**, not alternating blocks. `FULLCHECK=1` adds `-DB64CHECK` and checks all
four. `yolow64` validates every block. Under equal full validation, `yolow64`
is first at 64 KiB–4 MiB, tied at 16 KiB and about 2.1% behind at 8 KiB. These
are the final six-process medians after the exact-length short dispatch, the
pext padded tails and the tail-group change; the run before those changes was
19.66 / 19.61 / 20.12 / 20.18 / 20.15 / 17.35.

Short decode, ns/call:

| Input | 8 B | 12 B | 16 B | 24 B | 32 B | 48 B | 64 B | 96 B | 128 B |
|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| yolow64 | 14.26 | **7.00** | **8.53** | **10.14** | 19.14 | **12.20** | 23.18 | **15.68** | **17.18** |

The exact-length fast path shipped: 16-character (12 B), 32-character (24 B),
64-character (48 B) and 128-character (96 B) inputs with no trailing padding
bypass the general whitespace/length/padding dispatch and call `decode16` or
`avx2::decode` directly. `Codec::decode` is `#[inline]`, so the eight-library
harness can inline the dispatch. Six-process medians are in
[`../benchmarks/results/8way_short_decode.csv`](../benchmarks/results/8way_short_decode.csv). The fast path
is first at 12, 16, 24, 48, 96 and 128 B in the recorded eight-library table.

The 24-character (16 B) case now uses a single `avx2+bmi2` function: the first
16 characters run the 128-bit table translation/pack and the last six run a
BMI2 `pext` tail in the same call. It improved from 16.69 to 8.53 ns and is now
first (Turbo 8.59). The 44-character (32 B) case uses `decode32` plus `pext8`
and `pext3`; it improved from 19.67 to 19.14 ns. The 172-character (128 B) case
is a single function too: the first 160 characters use the hand-scheduled hash
loop for four blocks plus one block, and the last twelve use `pext8`/`pext3`.
It improved from 27.54 to **17.18 ns** and is first (leader 19.20). The
88-character case still uses the scalar tail; a copy-to-stack plus padded-block
prototype was slower and was not shipped.

An implementation bug during this round was instructive: the cached
`bmi2_available()` was accidentally written as
`get_or_init(|| bmi2_available())`, a self-recursive initializer. The first
len44 call hung the scalar test at size 32. Changing it to
`is_x86_feature_detected!("bmi2")` fixed the hang; the 24/44 kernels themselves
were correct.

## Cold decode

`decode_cold` rotates through a 64 MiB arena, matching the encode cold test.
Three-process medians, GB/s:

| Decoded input | Warm | Cold | Cold/warm |
|---:|---:|---:|---:|
| 4 KiB | 17.89 | 5.80 | 0.32 |
| 64 KiB | 19.76 | 7.57 | 0.38 |
| 1 MiB | 19.93 | 8.44 | 0.42 |
| 4 MiB | 19.10 | 8.71 | 0.46 |
| 16 MiB | 8.49 | 8.10 | 0.95 |

Warm repeated calls overstate one-shot decode by 2–3x from 4 KiB to 4 MiB. At
16 MiB both modes are memory-bound and converge near 8 GB/s. The cold table is
the more relevant number for socket/file decoders; the warm table is the right
target for cache-resident repeated use.

## Shipped: speculative validation and hash loop

The four-block assembly loop previously tested each pair of blocks and branched
to a per-pair error locator before storing the previous pair. The new loop:

1. validates each pair into an error vector;
2. ORs that vector into one `ymm10` accumulator without branching;
3. transforms and stores every block unconditionally;
4. tests the accumulator once after the loop; if any block was invalid it
   returns a rescan sentinel.

Speculative stores are legal because the API already documents that a
caller-provided decode output may contain partial bytes on error. The Rust
wrapper handles the sentinel by scanning for the first non-alphabet byte and
returning that offset, so error variants and offsets are unchanged.

Speculative validation was stage one. Stage two replaced the membership/roll
sequence with the clean-room hash network and moved **pair, quad and pack into
registers**, so the loop has no constant-memory operands. Six independent
processes, `yolow64` decode medians:

| Input | Before | Speculative | Final hash loop | Final change |
|---:|---:|---:|---:|---:|
| 8 KiB | 18.31 | 18.97 | **19.66** | +7.4% |
| 16 KiB | 18.53 | 19.28 | **19.61** | +5.8% |
| 64 KiB | 18.76 | 19.65 | **20.12** | +7.3% |
| 256 KiB | 19.08 | 19.95 | **20.18** | +5.8% |
| 1 MiB | 19.18 | 19.95 | **20.15** | +5.1% |
| 4 MiB | 16.62 | 17.12 | 17.35 | +4.4% (noisy) |

The final 8 KiB–1 MiB range is stable across every process. The 4 MiB cell is
still dominated by run-to-run variance.

## Experiments that did not ship

1. **32-byte stores plus `vpermd` in the assembly loop.** The current assembly
   writes each 24-byte block with two overlapping 16-byte stores. A single
   32-byte store needs a contiguous 24-byte layout, which requires a `vpermd`.
   Measured: 64 KiB 18.52 vs 18.76 GB/s, 1 MiB 18.81 vs 19.18. The extra
   shuffle-port pressure cancels the store saving. Reverted.
2. **16-byte decode fast path.** Decoding the padded 24-character input with
   `decode16` plus a scalar tail improved 16 B from 19.68 to about 15.0 ns, but
   the added branch and code shifted the scalar short paths: 8–128 B regressed
   by 0.2–2 ns. The branch was moved out of line and `#[inline(never)]`; the
   regression remained. Reverted.
3. **Eight-character scalar unroll with one u16 and one u32 store.** Replacing
   six byte stores with two wide stores did not win: 8–128 B were flat or
   0.2–1.7 ns slower. The compiler already schedules the byte stores well, and
   the extra lookups increased register pressure. Reverted.
4. **`decode_any` for every length ≥32.** The Rust single-block loop is faster
   than the assembly at very short lengths, but not at 64 KiB: 17.05 vs
   18.76 GB/s at 64 KiB, 15.68 vs 19.18 at 1 MiB, 12.16 vs 16.62 at 4 MiB.
   The assembly remains the large-input path.
5. **Prefetch distance 2048/4096 in the assembly.** 64 KiB and 1 MiB were flat;
   4 MiB moved by ±0.3 GB/s, within noise. Kept 1024/1088.
6. **Clean-room hash translation/validation (shipped in stages).** We
   generated independent `delta_asso`/`delta_values` and
   `check_asso`/`check_values` tables for both alphabets with a constraint
   search, then wrote a Rust AVX2 kernel using the `avg`/`adds` hash network.
   The Rust version was slower (64 KiB 18.26, 1 MiB 16.18). A four-block hash
   assembly with constants in memory matched the old loop (64 KiB 19.63,
   1 MiB 19.75). Moving pair/quad/pack into registers shipped: 8 KiB 19.66,
   16 KiB 19.61, 64 KiB 20.12, 256 KiB 20.18, 1 MiB 20.15. A Rust eight-block
   hash loop was slower (64 KiB 19.28, 1 MiB 17.90) because LLVM spills. The
   remaining limit is the eight-chain register schedule, not the hash math.

7. **Two-group 4-block unroll.** Duplicating the four-block hash body so one
   loop iteration handles eight blocks halved the loop-control overhead but
   did not win: six-process medians were 8 KiB 19.63, 16 KiB 19.60, 64 KiB
   20.05, 1 MiB 20.19 GB/s, flat to slightly below the four-block loop. The
   shuffle port, not loop control, is the limit. Reverted.
8. **Self-validating translation tables.** We searched for one pair of
   `delta_asso`/`delta_values` tables whose output is 0..63 for all 64 valid
   bytes and ≥64 (or bit-7 set) for all 192 invalid bytes, so that
   `vpsubusb`/`vpand` could replace the separate check network. An exhaustive
   DFS over the `avg`/`add` table form and z3 searches over two-shuffle
   additive, XOR, OR and composed forms all returned unsatisfiable. The
   information bottleneck is the two 16-entry tables: they cannot both encode
   the five valid offsets and route every invalid byte out of range. Not
   shipped; the separate speculative check remains.

9. **Pext tails for 88/172 characters.** The first 172 attempt used a
   `decode160` helper plus `pext8`/`pext3`; it was flat at 128 B
   (27.05→27.21) because of helper calls. The shipped version is a single
   `avx2+bmi2` function with the hand-scheduled hash loop for four blocks plus
   one block, then `pext8`/`pext3`: 128 B **17.18 ns**, first. The 88-character
   case (`decode64` + `decode16` + `pext6`) improved 64 B 22.51→21.64 but
   shifted other short cells and lost #1 at 24/48 when tried; it remains
   unshipped.
10. **Entry alignment and dead move.** Aligning `decode_hash4` to 64 bytes and
    removing the unused `mov %rdi, %r9` was flat at 8 KiB (19.63–19.71 GB/s).
    Reverted.
11. **Isolating the 24-character kernel.** Two layouts were tried to stop
    the single-function 24 kernel from shifting the other short paths. A
    `decode_common` wrapper (16 B 7.86, 12 B 7.43, 24 B 10.71, 48 B 13.21,
    8 KiB 19.67 GB/s) made 16 B faster but dropped 24/48 B to second place. A
    tiny out-of-line dispatcher kept the old no-pad layout but pushed 16 B to
    15.96 ns because of the extra call. Kept the inline before-no-pad version:
    16 B 8.53 (first), 12/24/48/96 B first, 8 KiB 19.59.
## What the fastest loop does

The 26 GB/s default Turbo build uses a **256-byte (eight-block) main loop**,
but its DS128 groups check only the first of four blocks (`CHECK1` is empty);
the other three are translated without validation. Its `FULLCHECK=1` build is
19.36 GB/s at 64 KiB in the verification rebuild (19.71 in the earlier run),
below the shipped `yolow64` hash loop. The loop shape is still instructive:

- eight independent hash chains are live at once;
- seven constants (two asso/value pairs, pair, quad, pack) plus an error
  accumulator occupy the remaining YMM registers, so the loop runs with all 16
  registers and no spills;
- validation is speculative: `vpaddsb` results are ORed into one accumulator
  and tested once after the loop;
- each 32-byte block still uses two 16-byte stores.

Its weak points, from our measurements and the loop shape:

- no input prefetch in the hot loop; our 1024/1088 prefetch is an edge;
- a separate 16-byte tail loop and a conservative `64 + 2*128 + 4` bound;
- no non-temporal path for large outputs;
- the schedule is hand/compiler-tuned around 16 registers, which is why a
  straightforward Rust port does not reproduce it.

Our experiments around that structure:

| Variant | 64 KiB | 1 MiB |
|---|---:|---:|
| shipped hash loop, all constants in registers | **20.12** | **20.15** |
| previous assembly + speculative validation | 19.66 | 19.96 |
| four-block hash, constants in memory | 19.63 | 19.75 |
| Rust eight-block hash loop | 19.28 | 17.90 |
| Rust one-block hash loop | 18.26 | 16.18 |

Moving pair/quad/pack from memory operands into registers was worth about 2%.
The Rust eight-block loop lost to spills/scheduling, so the four-block schedule
was kept.

## Why the gap is now small

The shipped bulk path is the four-block hash loop: `vpsrld`/`vpavgb` hash,
`vpaddsb` speculative check, `vpmaddubsw`/`vpmaddwd`/`vpshufb` pack, and
two 16-byte stores per block. Pair/quad/pack stay in registers, so the loop has
no constant loads. The caller now runs `(blocks - 1) / 4` assembly groups, so
the four-byte spill always lands in at least one valid tail block; this moved
up to three blocks out of the Rust tail and improved 8 KiB from 19.63 to
19.78 GB/s.

Under full validation, yolow64 is first from 64 KiB through 4 MiB: 20.08 vs
19.36 at 64 KiB, 20.16 vs 16.76 at 256 KiB, 20.17 vs 16.68 at 1 MiB and 17.06
vs 13.89 at 4 MiB. It is tied at 16 KiB (19.54 vs 19.68) and about 2.1% behind
at 8 KiB (19.64 vs 20.06). 
