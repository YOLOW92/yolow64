# Changelog

## 0.3.0 — 2026-11-12

Architecture decoupling, new short/tail SIMD decode kernels, and documentation overhaul.

### Decode Kernels

- Added dedicated padded short-input SIMD kernels in `src/avx2/decode.rs`:
  - `decode12` (8 B payload, 12 chars): safe 11-byte + `'A'` load into 128-bit XMM, single-pass hash translation/validation/pack, and `_mm_extract_epi8::<8>` trailing padding-bit check (**5.47 ns**, 2.61× faster than v0.2).
  - `decode24` (16 B payload, 24 chars): overlapping `0..16` and `8..24` (`"=="` replaced by `"AA"`) combined into a single 256-bit AVX2 vector pass with `_mm_extract_epi8::<4>` padding-bit check (**5.69 ns**, 1.50× faster).
  - `decode44` (32 B payload, 44 chars): fused 256-bit AVX2 block (`0..32`) + overlapping 128-bit XMM block (`28..44`) sharing LUT registers and a single `vptest` (**6.89 ns**, 2.78× faster).
  - `decode88` (64 B payload, 88 chars): fused 2×256-bit AVX2 blocks (`0..64`) + 128-bit XMM block (`64..80`) + BMI2 `pext6_be` tail (`80..86`) (**10.96 ns**, 2.11× faster).
- Added fused bulk + tail kernels `decode_hash4_tail44` (`groups * 128 + 44` chars, covering 128 B and 8 KiB) and `decode_hash4_tail88` (`groups * 128 + 88` chars, covering 16 KiB–4 MiB), lifting 8 KiB decode to **20.41 GB/s** and 64 KiB–1 MiB decode to **20.36–20.40 GB/s** (#1 across all 9 short sizes and all 6 full-validation bulk sizes).

### Architecture & Safety Fixes

- Decoupled monolithic `src/lib.rs` and `src/avx2.rs` into focused modules: `src/cpu.rs`, `src/tables.rs`, `src/encode.rs`, `src/decode.rs`, `src/stream.rs`, `src/parallel.rs`, and `src/avx2/{mod,encode,decode,whitespace,lab}.rs`.
- Fixed `Decoder::feed` partial buffer fill bug where a buffered 1–3 byte prefix was not completed before returning when the new strict chunk had `< 4` bytes.
- Fixed `decode_parallel_with_threads` silently clamping `max_threads` to 2 and documented that parallel chunk errors may leave partial output in caller slices.
- Removed dead constant loads from the `decode_hash4` assembly loop and isolated experimental kernels into `#[cfg(test)] mod lab`.

## 0.2.1 — 2026-11-11

Documentation-only patch.

- Point installation at crates.io (`yolow64 = "0.2"`).
- No code, validation or benchmark changes.

## 0.2.0 — 2026-11-11

Performance and compatibility release.

### Decode

- Replaced the bulk decode kernel with a four-block AVX2 hash loop using
  clean-room translation/validation tables, speculative validation, and
  per-quad/pack constants held in registers.
- Added a BMI2 `pext` tail for padded 24- and 172-character inputs, and a
  single-call 24-character `avx2+bmi2` kernel. Exact-length no-padding
  16/32/64/128-character inputs bypass the general dispatch.
- Short-decode cells are first at 12, 16, 24, 48, 96 and 128 bytes in the
  recorded eight-library run; the bulk path is first from 64 KiB through 4 MiB
  under equal full validation.
- Full validation is retained for every block. Padding-bit checks reject
  non-zero trailing bits.

### Encode

- Added adaptive non-temporal stores above `3/7` of the detected LLC, a
  streaming encode API, and a memory-bounded `encode_to_writer`.
- Short 8/16/32-byte fast paths use a single conversion and overwrite the
  padding characters instead of running the generic tail.
- The optional `parallel` feature uses cache-aware workers; large inputs stay
  on the single-thread streaming path when that is faster.

### Compatibility and safety

- AVX2 and BMI2 are detected independently. BMI2 `pext` paths run only when
  both features are present; AVX2-only and scalar CPUs fall back without
  executing BMI2 instructions.
- Added exact-output canary tests for caller-provided decode buffers and
  streaming encode buffers, plus explicit non-zero-trailing-bit tests and a
  test-only no-BMI2 fallback switch.
- MSRV is Rust 1.89; CI covers Linux x86-64, aarch64 cross-check and Windows
  cross-check.

### Benchmarks and records

- The internal R&D record, including rejected kernels, moved to
  [`research/`](research/README.md).
- Benchmark numbers are snapshots from a shared Intel i7-10700; see
  [`benchmarks/README.md`](benchmarks/README.md) and
  [`research/decode.md`](research/decode.md).
- Turbo-Base64 comparisons in the release tables use `FULLCHECK=1` for equal
  validation. Its default build checks only the first of four blocks in the
  256-byte hot loop; the verification is in
  [`research/turbo-check.md`](research/turbo-check.md).
