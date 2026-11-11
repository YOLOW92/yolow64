# R&D record

This directory keeps the full engineering record behind the shipped paths: the
measurements, the kernels that won, and the kernels that were tried and
reverted. The rejected experiments are intentionally preserved; they document
the design space and the reasons a path was not taken.

## Contents

- [`decode.md`](decode.md) — decode history, cold-buffer data, the shipped
  hash loop, the `pext` short kernels, and the rejected decode kernels.
- [`encode-counters.md`](encode-counters.md) — instruction/cache counters,
  cold-buffer encode, the `3/7` LLC streaming threshold, prefetch sweeps, and
  memory measurements.
- [`encode-comparison.md`](encode-comparison.md) — the recorded 22-way encode
  comparison and the post-update notes.
- [`turbo-check.md`](turbo-check.md) — build-flag, dispatch-target, and
  disassembly verification for Turbo-Base64's default (25%) and `FULLCHECK`
  (100%) validation configurations.
- [`experiments.md`](experiments.md) — the chronological experiment log,
  including rejected kernels and API measurements.
- [`turbo_check.c`](turbo_check.c) — standalone driver used for the validation
  build comparison.
- [`figures/generate.py`](figures/generate.py) — regenerates the public SVGs
  in [`../docs/figures/`](../docs/figures/) from the recorded CSV data.
- [`release-0.2.0.md`](release-0.2.0.md) — final build, benchmark and safety
  verification record for the 0.2.0 release.

Raw CSV data lives in [`../benchmarks/results/`](../benchmarks/results/). The
public benchmark method and the summary tables live in
[`../benchmarks/README.md`](../benchmarks/README.md).

## Rejected or deferred ideas kept in the record

- 32-byte stores plus `vpermd` in the assembly loop.
- The earlier padded 16-byte fast path.
- Eight-character scalar unroll.
- `decode_any` for every length ≥32.
- Two-group / eight-block unrolls that did not beat the shuffle-port limit.
- Self-validating translation tables (proved unsatisfiable for the tested
  two-shuffle forms).
- Entry alignment and dead-move micro-tuning.
- Isolating the 24-character kernel in `decode_common` or a tiny out-of-line
  dispatcher (kept inline because the call cost was larger than the layout
  benefit).

## Reading the numbers

The measurements were taken on an Intel Core i7-10700 (Comet Lake) with AVX2
and BMI2, no AVX-512, on a shared Linux machine. Absolute numbers are snapshots;
the 8–128 B and 64 KiB rows are the most stable, while 1–4 MiB is noisy. The
external eight-library columns are a recorded run and were not re-ranked after
each change. Full-validation decode comparisons use `FULLCHECK=1`; Turbo's
default build checks one of four blocks in its 256-byte hot loop (25%).
