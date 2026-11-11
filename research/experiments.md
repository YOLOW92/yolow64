# Experiment log

This file records the optimisation experiments behind the current implementation, including attempts that were rejected. Timings are exploratory and were taken on the shared i7-10700 used for the main benchmark; the committed 8-way comparison is the authoritative performance table.

## Method

- Paired, interleaved samples: every variant is timed in the same process with the variant order rotated each round.
- A cell is the median of the per-round samples. For paired comparisons, the median of the per-round differences is reported with a deterministic 10,000-resample bootstrap 95% interval.
- Correctness is checked before timing. A variant that changes output bytes, error offsets or accepted inputs is rejected regardless of speed.
- The microbenchmark exercises valid, whitespace-free input; the error path is checked by the differential and mutation tests.

Run the decode-variant lab with:

```sh
cargo test --release decode_variant_lab -- --ignored --nocapture
```

## Decode experiments

Latest lab run, ns/call (lower is better). `public` is `Codec::decode`, `any` is `avx2::decode_any`, `batch4` is the four-block batch-validation kernel, and `main` is the hand-scheduled `avx2::decode`.

| Encoded bytes | public | any | batch4 | main | batch4 − any, 95% CI |
|---:|---:|---:|---:|---:|---:|
| 32 | 11.45 | 6.76 | 7.93 | 7.19 | +1.17 [+1.06, +1.22] |
| 64 | 12.17 | 7.65 | 8.97 | 9.04 | +1.32 [+1.28, +1.35] |
| 96 | 13.37 | 13.97 | 10.66 | 10.94 | −3.28 [−3.35, −3.28] |
| 128 | 15.41 | 11.00 | 12.54 | 12.58 | +1.50 [+1.45, +1.56] |
| 256 | 21.62 | 16.98 | 19.06 | 21.39 | +2.10 [+2.08, +2.12] |
| 512 | 31.56 | 26.37 | 32.54 | 30.21 | +6.18 [+6.10, +6.28] |
| 1024 | 52.57 | 51.25 | 59.73 | 53.29 | +8.46 [+8.42, +8.50] |
| 1536 | 83.42 | 78.19 | 88.60 | 71.95 | +10.48 [+10.38, +10.55] |
| 2048 | 101.97 | 102.57 | 116.62 | 95.24 | +13.99 [+13.89, +15.40] |
| 4096 | 184.13 | 202.53 | 246.51 | 186.95 | +43.77 [+42.86, +44.18] |
| 65536 | 2632.08 | 2990.86 | 3729.61 | 2592.02 | +720.47 [+716.13, +740.58] |

Conclusions from this run:

- **Four-block batch validation was rejected.** It saves a branch but increases live vector state; LLVM spills, and it is slower from 128 bytes upward. The Rust `decode_any` already batches two blocks and the hand-written loop batches two.
- **`decode_any` wins below roughly 1.5 KiB; the hand-scheduled kernel wins above.** The public threshold was set to 1800 encoded bytes so that a 1 KiB original payload (about 1.37 KiB encoded) still uses `decode_any`, while 1.5 KiB and larger use the hand-scheduled kernel.
- **`vptest` is neutral to slightly positive.** The hand-written loop was changed from `vpmovmskb` + scalar test to `vpminub` + `vpcmpeqb` + `vptest`, removing two instructions per two-block check. The change was kept because it is simpler and never measurably slower.
- **Removing the assembly prefetches was inconclusive.** At 4 KiB and 8 KiB it was slightly faster, at 64 KiB it was slower (about 2–3%); the prefetches were kept because the large-size case is more representative of streaming workloads. This needs a quieter machine to settle.

### Sixteen-byte kernel

A 16-byte SSSE3/AVX2 kernel was added for the final 16-byte group and for exactly-16-byte prefixes.

- It improved a 12-byte original payload (16 encoded bytes) from about 15.4 to 9.6 ns.
- It was slower for a 16-byte original payload (24 encoded bytes), because the kernel call plus an 8-byte scalar remainder cost more than a single table-driven pass.
- It was therefore restricted to `full_end == 16`; other short tails stay scalar.
- Padding a 12/16/24-byte tail to a full 32-byte vector and calling `decode_any` was also tried. The stack copy and extra output copy made it slower than scalar; rejected.

### Validation and error strategy

- The SIMD validation produces a zero byte for an invalid character. Two blocks are combined with `vpminub`; a zero at any position means at least one block is invalid. The rare failure path re-checks the vectors to find the first invalid byte.
- Batch sizes larger than two were measured and rejected (see `batch4`). The expected-cost argument favours large batches only if the extra live state does not spill; on this target it does.
- Exact error offsets are covered by `invalid_byte_offsets_survive_simd_tails`, including values around the 16/32/2KiB boundaries.

## Encode experiments

- The sub-24-byte encode tail previously used four 64-byte table lookups per three input bytes. A 12-bit pair table (two lookups per three bytes) improved 8 B from about 10.3 to 9.0 ns and 16 B from about 15.9 to 12.4 ns. The table is 8 KiB per alphabet and stays in L1.
- A 12-byte SSE chunk (16 output characters) is now used for the first 12 bytes of 12–23-byte inputs. For exactly 12 B it is the fastest short path measured (7.3 ns vs 7.7 ns for Turbo-Base64). For 13–23 B the remaining 0–11 bytes go through the scalar pair table; this was originally about 10.3 ns at 16 B.
- Single-load fast paths now handle exactly 8, 16 and 32 bytes. They reuse the 24-byte AVX2 conversion with zero extension, then overwrite the final padding characters. In the stable eight-library harness, 8 B is 6.02 ns (was 9.28), 16 B 6.20 (was 10.27) and 32 B 7.29 (was 12.98); 12 B remains 7.35. The 16 B scalar-tail anomaly is closed. yolow64 is fastest at 8, 12, 16, 24, 32, 48 and 96 B in the eight-library harness; in the broader 22-way snapshot tape-base64 is still slightly ahead at 24 B. Turbo-Base64 wins 64 B and 128 B.
- For 24–96 B, two 16-byte loads per group avoid the cache-line-crossing 32-byte loads. This reduced 96 B from about 13.7 to 12.2 ns. Above 96 B the extra load uops dominate, so the normal 32-byte-load kernel is used.
- For 4 KiB and above, an eight-block sequential loop with a precomputed chunk count (192 input bytes / 256 output bytes per iteration) improved 4 KiB by about 1%, 8 KiB by about 3%, 64 KiB by about 4%, 256 KiB by about 3% and 1 MiB by about 3%. Splitting the loop into a prefetch prefix and a no-prefetch remainder removed the per-iteration prefetch branch.
- Prefetch distance was swept from 0 to 8192 bytes. At 4 KiB no prefetch was slightly better; at 64 KiB–1 MiB a distance of 4–8 KiB was best. The final constant is 8192, with no prefetch for inputs shorter than that.
- The complete-block overlap already removes the scalar tail for encode.

Rejected variants, all measured with paired interleaved rounds:

- All-live eight-block unroll: register spills; slower from 128 B upward.
- Four two-block phases / phased 16-byte loads: slower at every measured size.
- Compile-time mixed 16/32-byte loads for the crossing windows: slower at 256 KiB and 1 MiB.
- Hand-written AVX2 load16 assembly: 8–10% slower than the compiler-generated 32-byte-load loop.
- `prefetchw` on the output: no consistent win; disabled.
- Hand-written encode loop was not attempted further because the remaining gap is in the cache-resident 4–64 KiB range and the compiler's sequence is already close to the instruction minimum.
- A diagnostic build with `RUSTFLAGS='-C target-cpu=native'` improved the 8–64 KiB range by roughly 2–4%, but still left yolow64 about 5–11% behind Turbo-Base64 there. The library cannot force a target CPU through Cargo; this is recorded as a possible build-time tuning option, not a source-level fix.

Current position after this round: encode is fastest in the 8-way table at 12 B, 24 B, 48 B, 96 B, 256 B, 512 B and 1 MiB. It is about 5–8% behind Turbo-Base64 at 4–64 KiB and within run-to-run noise at 4 MiB. With adaptive streaming stores and wide prefetch it reaches 13.0 GB/s at 16 MiB (13.0–13.3 across campaigns), above the recorded 10.96 GB/s for tape-base64 and 10.70 GB/s for base64-turbo. Decode remains the larger gap.

## CRLF experiments

- Original filter used a scalar `copy_nonoverlapping` per run of non-whitespace bytes. 4 MiB CRLF decode was about 1.21 ms.
- An eight-byte compaction LUT (256 × 8-byte shuffle masks plus popcounts) reduced 4 MiB to about 0.91 ms.
- A 76-column MIME fast path (copy 76-byte lines, batch strict decode) reduced 4 MiB to about 0.65–0.71 ms and 1 MiB to about 0.12 ms. It now beats encodify at 4 MiB and is within about 10–15% at 64 KiB–1 MiB.
- The fast path is guarded by a structural check. Any mismatch, invalid byte or output shortage falls back to the generic path, preserving error offsets.

## API and streaming experiments

- Removing the zero-fill from `append_encode`, `encode_to_string`, `decode_to_vec` and `append_decode` made owned/append APIs track the slice APIs at 64 KiB and 1 MiB.
- Bulk `Decoder::feed` plus `first_special` (AVX2 search for `=`/whitespace) reduced a 1 MiB strict message fed in 4 KiB chunks from about 0.14 GB/s to about 9 GB/s. The one-shot path improved by about 65x.
- The generic whitespace streaming path is still byte-wise. It was not optimised further because the MIME/CRLF caller can feed filtered data and because preserving exact offsets across chunks is subtle.

## Parallel experiments

- `std::thread::scope` with 1/2/4/8/16 workers was measured per size in separate processes. Early measurements without streaming stores showed a modest gain around 2–4 MiB, while 8 MiB and larger were memory-bandwidth-bound.
- After the streaming-store path was added, the crossover moved. A three-process sweep with `parallel_encode` gives: 3 MiB single-thread 19.6 GB/s versus 16.9 with four workers (thread creation loses); 4 MiB 18.1 versus 22.5 (+24%); 6 MiB 14.0 versus 17.2 (+23%); 6.5 MiB 12.5 versus 15.1 (+21%); 7 MiB single-thread NT 15.4 versus 13.4 with four cached workers; 8 MiB 15.0 versus 10.9; 16 MiB 12.4 versus 7.8.
- The automatic encode policy is therefore: at least 4 MiB, up to four workers while `input + output` fits the detected LLC, and one worker above the LLC (where the single-thread streaming path wins). Decode keeps the conservative two-worker policy and a 7/8-LLC margin until its own round.
- Encode chunk boundaries are rounded down to a multiple of four groups. Four groups produce 16 output bytes, so every worker after the first starts on a 16-byte boundary and remains eligible for streaming stores.
- `available_parallelism` is cached in a `OnceLock`, and the automatic API returns directly to `encode` below 4 MiB. Without these, the wrapper's syscall and validation were visible on 2–3 MiB calls.
- A persistent thread pool was not implemented. It could lower the 2–3 MiB crossover, but adds global state, contention and lifecycle questions; it should be measured separately.

## Streaming-store experiments

- Non-temporal stores were measured with the 8-block split kernel. The crossover is the LLC working-set boundary: input plus output is `7/3` of the input, so the input floor is `3/7` of the LLC. On the 16 MiB-L3 host that is 6.86 MiB.
- Below the floor cached stores win: at 4 MiB, 21.0 GB/s cached versus 15.6 GB/s with 16-byte NT stores in the same sweep. Above it NT stores win: 8 MiB goes from 9.6 to 15.2 GB/s, and 16 MiB from 7.2 to 12.5 GB/s (1.74x in the isolated sweep).
- The production dispatch reads CPUID leaf 4, AMD leaf `0x8000_001d`, or AMD `0x8000_0006`, accepts only 64 KiB–2 GiB, and requires a 16-byte-aligned output. Unknown LLC or unaligned output falls back to cached stores.
- Two 16-byte `movntdq` stores per 32-byte vector avoid a 32-byte alignment requirement. The 32-byte `vmovntdq` variant was similar and not consistently faster.
- Prefetch coverage: one loop iteration consumes 192 input bytes, three 64-byte lines, but the original loop issued only one prefetch per iteration. Issuing three prefetches (offsets 0/64/128) changed 10 MiB from 14.2 to 15.9 GB/s, 12 MiB from 13.3 to 14.9, 14 MiB from 12.4 to 13.7 and 16 MiB from 12.9 to 13.2. The 8 KiB look-ahead is within noise of the best distance, and no prefetch is 10–15% worse.
- A targeted three-process `encode_all --lib yolow64` run gives 13.0 GB/s at 16 MiB (13.0–13.3 across campaigns), versus 7.36 GB/s before (1.77x). Counter-level details are in `benchmarks/encode-counters.md`.

## Cold-buffer and one-shot experiments

- Warm repeated-call benchmarks reuse one input/output pair. A 64 MiB rotating arena with scattered slots measured the one-shot case: cold `encode` is 5.7–8.0 GB/s from 4 KiB to 4 MiB, about 3x below the warm 20–22 GB/s, and 11.7 GB/s at 16 MiB.
- The gap is the output read-for-ownership. Forcing 16-byte NT stores gave 7.34 GB/s at 4 KiB (versus 5.85 cached), 11.53 at 64 KiB, 12.46 at 1 MiB, 12.72 at 4 MiB and 11.69 at 16 MiB. At 1 KiB NT was a tie (3.65 versus 3.63) and at 256 B it was worse (1.61 versus 2.80), so the one-shot threshold is 4 KiB.
- The public policy is therefore split by API: `encode` keeps the cache-friendly `3/7 LLC` rule for reused/reread buffers; `encode_streaming` uses NT from 4 KiB when the output is 16-byte aligned, for sockets, files and compressors. Cold `encode_streaming` reaches 7.9–13.4 GB/s from 4 KiB to 4 MiB, 1.39–1.68x over cold `encode`; at 16 MiB both converge.
- The cold benchmark is `benchmarks/compare/src/bin/encode_cold.rs`; three-process medians are in `../benchmarks/results/encode_cold_combined.csv`.
- `encode_to_writer` processes 48 KiB multiples of three through a fixed 64 KiB buffer. For a 256 MiB input, peak RSS is 264,256 KiB versus 613,640 KiB for `encode_to_string`, a 349,384 KiB (about 341 MiB) reduction; the cost is one copy and one `write_all` per 64 KiB.

## Decode round status

- Cold-buffer decode drops from 17.9–19.9 GB/s warm to 5.8–8.7 GB/s at 4 KiB–4 MiB; at 16 MiB both converge near 8 GB/s. The one-shot path is memory-bound, not instruction-bound.
- Speculative validation shipped: the assembly loop now ORs error vectors into a single accumulator, stores unconditionally, and tests once after the loop; the Rust wrapper rescans for the first invalid byte only on the error path. Six-process medians improved 8 KiB 18.31→18.97, 16 KiB 18.53→19.28, 64 KiB 18.76→19.65, 256 KiB 19.08→19.95 and 1 MiB 19.18→19.95 GB/s (+3.6–4.7%). The 4 MiB cell stayed noisy (12.4–18.4 GB/s), so it is not claimed as a stable win.
- The remaining warm 8–64 KiB gap to Turbo-Base64 is a packing/shuffle gap, not a branch gap. A single-block 32-byte store plus `vpermd` did not beat the two overlapping 16-byte stores (64 KiB 18.5 vs 18.8 GB/s); `decode_any` for all sizes was slower; prefetch tuning was within noise.
- An earlier padded 16-byte decode fast path improved 16 B from 19.7 to 15.0 ns but shifted the scalar short paths and regressed 8–128 B by 0.2–2 ns, so it was reverted. An eight-character scalar unroll was also reverted.
- Exact-length decode dispatch shipped. No-padding inputs of 16, 32, 64 and 128 characters now bypass the general whitespace/length/padding dispatch and call `decode16` or the AVX2 block kernel directly; `Codec::decode` is `#[inline]` so the harness inlines it. Six-process medians: 12 B payload 7.00 ns, 16 B 8.53, 24 B 10.14, 48 B 12.20, 96 B 15.68, 128 B 17.18. These are first at 12, 16, 24, 48, 96 and 128 B in the recorded eight-library table.
- The padded 24- and 44-character cases now use a BMI2 `pext` tail. A 256-byte decode table maps each character to its six-bit value or 0xff; eight scalar lookups build a 64-bit word, `pext` compacts the six-bit fields into a dense 48-bit group, and shifts write the bytes. The 24-character case uses a single `avx2+bmi2` function (128-bit head plus `pext6`): 16.69→8.53 ns, first. The 44-character case uses `decode32` plus `pext8`/`pext3`: 19.67→19.14 ns. The 172-character case uses the hand-scheduled hash loop for four blocks plus one block, then `pext8`/`pext3`: 27.54→17.18 ns, first. The 88-character case still uses the scalar tail; a copy-to-stack plus padded-block prototype was slower and was not shipped.
- The bulk hash caller now leaves one to four blocks for the Rust tail, `(blocks - 1) / 4` assembly groups, instead of always leaving four. The assembly's four-byte spill lands in the first tail block; this moved up to three blocks out of the slower Rust tail and improved 8 KiB from 19.63 to 19.78 GB/s. The final six-process number after the pext dispatch branches is 19.64 GB/s, still 2.1% behind the fully validating 20.06.
- A simple two-group unroll of the four-block hash loop (eight blocks per iteration) did not win: 8 KiB 19.63, 16 KiB 19.60, 64 KiB 20.05, 1 MiB 20.19 GB/s, flat to slightly below the four-block loop. The shuffle port, not loop control, is the limit; reverted.
- Self-validating translation tables were searched as an alternative to the separate check network: one `delta_asso`/`delta_values` pair whose output is 0..63 for all valid bytes and ≥64 (or bit-7 set) for all invalid bytes, so `vpsubusb`/`vpand` could replace two check shuffles. An exhaustive DFS over the `avg`/`add` form and z3 searches over two-shuffle additive, XOR, OR and composed forms all returned unsatisfiable. The two 16-entry tables cannot encode the five valid offsets and route every invalid byte out of range. Not shipped.
- A clean-room hash translation/validation kernel was generated from independent constraint-search tables for both alphabets. The Rust AVX2 version was correct but slower than the hand assembly: 64 KiB 18.26 vs 19.67 GB/s, 1 MiB 16.18 vs 19.96, 4 MiB 12.05 vs 17.4. A hand-scheduled four-block hash assembly with memory constants matched the old loop (64 KiB 19.63, 1 MiB 19.75). Moving pair/quad/pack into registers removed the constant loads and shipped: 8 KiB 19.66, 16 KiB 19.61, 64 KiB 20.12, 256 KiB 20.18, 1 MiB 20.15 GB/s (+5–7% over the original decode). A Rust eight-block (256-byte) hash loop was slower (64 KiB 19.28, 1 MiB 17.90) because LLVM spills and cannot reproduce the reference register schedule. The Rust eight-block loop was not used; the four-block schedule stayed.
- Validation parity matters, and the earlier description was too generous to the default build. Turbo-Base64's `DS128` hot loop handles four 32-byte blocks: `CHECK0` (always enabled without `NB64CHECK`) covers the first, and `CHECK1` (enabled only by `-DB64CHECK`, i.e. `FULLCHECK=1`) covers the other three. Default therefore checks **one block in four (25%)**, not alternating blocks. A clean rebuild verified the makefile mapping (`FULLCHECK=1` adds `-DB64CHECK`), the default dispatch target (`tb64dec` -> `_tb64d` -> `tb64v256dec`, pointer equality), and the disassembly (`vpavgb` 17 default vs 26 full; the +9 is exactly three extra checks in each of three `DS128` expansions). With full validation, the 64 KiB number is 19.36 (26.90 default) and 1 MiB is 16.68 (20.57 default). Under full validation, yolow64's final shipped loop is first at 64 KiB–4 MiB (20.08/20.16/20.17/17.06 vs 19.36/16.76/16.68/13.89), tied at 16 KiB and about 2.1% behind at 8 KiB. Steps and raw counts: `turbo-check.md`; decode details: `decode.md`.

## Rejected or deferred ideas

- AVX-512 kernels: not implemented on request; no local hardware.
- NEON changes: no ARM hardware available for measurement; the existing NEON path is retained.
- 12/16/24-bit large lookup tables for short input: the 8 KiB pair table was the best measured trade-off; larger tables added L1 pressure without a clear win.
- Four-block batch validation in Rust: rejected due to spills.
- Padded small-input SIMD via a stack copy: rejected; the copy and scratch traffic made it slower than the scalar tail. The 24/44-character cases shipped with a BMI2 `pext` tail instead.
- Pext tails for 88/172 characters: the first 172 attempt (`decode160` + `pext8` + `pext3` across helper calls) was flat; the shipped single-function version uses the hash loop for four blocks plus one block and reaches 17.18 ns. The 88-character variant (`decode64` + `decode16` + `pext6`) improved 64 B 22.51→21.64 but shifted other short cells and was reverted.
- Function-entry alignment and the unused `mov %rdi, %r9`: flat at 8 KiB (19.63–19.71 GB/s). Reverted.
- Isolating the 24-character kernel: a `decode_common` wrapper made 16 B 7.86 ns but dropped 24/48 B to second place (10.71/13.21); a tiny out-of-line dispatcher pushed 16 B to 15.96 ns. The inline before-no-pad layout stayed: 16 B 8.53, 12/24/48/96 B first.
