# yolow64

[中文](docs/README.zh-CN.md)

A Base64 codec for Rust, with a small API and runtime-dispatched SIMD. It uses only the Rust standard library at runtime.

## Story

While building [PageSift](https://github.com/YOLOW92/pagesift), we needed Base64 for browser screenshots, MCP resources and proxy authentication. Those call sites made correctness, allocation control and real-world throughput equally important. yolow64 extracts that codec into an independent MIT-licensed library.

## At a glance

- Standard and URL-safe alphabets; padded and unpadded forms.
- Slice, append, owned-output, validation, streaming decode and data URL APIs.
- Optional `parallel` feature with cache-aware automatic worker selection.
- AVX2 bulk and BMI2 short-decode paths on supported x86-64 CPUs, detected independently; NEON on supported AArch64 CPUs; scalar fallback.
- Rust 1.89 or newer; `std` required. **No runtime third-party dependencies.**
- Independently measured results and runnable benchmark below. Short inputs do not always win.

## How it works

![Decode pipeline](docs/figures/decode-pipeline.svg)

*Bulk AVX2 block path and the short exact-length / BMI2 `pext` paths.*

![CPU dispatch and fallback](docs/figures/cpu-dispatch.svg)

*AVX2 and BMI2 are detected independently; the `pext` short paths run only when both are present.*

## Release notes

See [`CHANGELOG.md`](CHANGELOG.md) for the 0.2.0 performance, compatibility and safety changes, and [`research/`](research/README.md) for the complete experiment record including rejected kernels.

## Install

Available on crates.io:

```toml
[dependencies]
yolow64 = "0.2"
```

## Usage

```rust
use yolow64::{Alphabet, Codec, Decoder, STANDARD};

fn main() -> Result<(), yolow64::Error> {
    let encoded = STANDARD.encode_to_string(b"hello")?;
    assert_eq!(encoded, "aGVsbG8=");
    assert_eq!(STANDARD.decode_to_vec(encoded.as_bytes())?, b"hello");

    let url = Codec {
        alphabet: Alphabet::UrlSafe,
        padding: false,
        ignore_whitespace: false,
    };
    assert_eq!(url.encode_to_string(&[0xfb, 0xff])?, "-_8");

    let mut decoder = Decoder::new(STANDARD);
    let mut output = Vec::new();
    decoder.feed(b"aGV", &mut output)?;
    decoder.feed(b"sbG8=", &mut output)?;
    decoder.finish(&mut output)?;
    assert_eq!(output, b"hello");
    Ok(())
}
```

The example is also available as `cargo run --example basic`.

`STANDARD` uses the standard alphabet, requires padding when applicable, and rejects whitespace. Set `ignore_whitespace` to `true` to ignore **only** space, tab, CR and LF. It does not accept arbitrary MIME garbage. With `padding: false`, padding is forbidden; non-zero unused bits are rejected in either mode.

For preallocated buffers, `Codec::encoded_len` calculates the encode capacity, `Codec::encode` and `Codec::decode` return bytes written, and `Codec::validate` returns the exact decoded length. `append_encode` and `append_decode` operate on `Vec<u8>`; `encode_data_url` does not validate the MIME type. Decoding into a caller-provided buffer can leave partial output on error: discard it. A streaming `Decoder` can also have appended output before an error; discard it when the whole message must be valid. `InvalidByte` carries the byte and original input offset; other errors include `InvalidLength`, `InvalidPadding`, and `OutputTooSmall`.

## Performance

An eight-implementation comparison was run on 2026-11-11 on the same machine, with the same inputs, buffers and process. Full tables, raw per-round data and the exact configurations are in [`benchmarks/README.md`](benchmarks/README.md). The experiment log, including rejected kernels, is in [`research/experiments.md`](research/experiments.md). A broader 22-implementation encode-only comparison is in [`research/encode-counters.md`](research/encode-counters.md) and [`research/encode-comparison.md`](research/encode-comparison.md); the decode status and cold-buffer data are in [`research/decode.md`](research/decode.md). In the pre-update 22-way comparison `yolow64` ranked 3rd for short inputs, 2nd for 192 B–4 KiB, and 3rd for 8 KiB–16 MiB; it was first at 1 MiB. After the 8/16/32-byte fast paths it is fastest at 8, 12, 16, 24, 32, 48 and 96 B in the eight-library harness. The counter report adds per-call instruction counts, D1/LLd miss rates and the crossover points.

Environment: Intel Core i7-10700 (Comet Lake), AVX2/BMI2, no AVX-512, Linux x86-64, Rust 1.99.0, release fat LTO. The machine is shared and `schedutil` is active; the 8–128 B and 64 KiB rows are stable, while 1–4 MiB differences are noisy. Three independent processes were run for the external columns, each with seven rotated rounds; cells are medians of the three process medians. The `yolow64` decode cells are medians of six independent processes.

### Short inputs, ns/call (lower is better)

| Input | yolow64 enc | fastest enc | yolow64 dec | fastest dec |
|---:|---:|---:|---:|---:|
| 8 B | 6.02 | 6.02 (yolow64) | 14.26 | 7.90 (Turbo-Base64) |
| 12 B | 7.35 | 7.35 (yolow64) | **7.00** | **7.00 (yolow64)** |
| 16 B | 6.20 | 6.20 (yolow64) | **8.53** | **8.53 (yolow64)** |
| 24 B | 9.01 | 9.01 (yolow64) | **10.14** | **10.14 (yolow64)** |
| 32 B | 7.29 | 7.29 (yolow64) | 19.14 | 10.36 (base64-simd) |
| 48 B | 9.95 | 9.95 (yolow64) | **12.20** | **12.20 (yolow64)** |
| 64 B | 13.41 | 12.11 (Turbo-Base64) | 23.18 | 12.81 (Turbo-Base64) |
| 96 B | 11.43 | 11.43 (yolow64) | **15.68** | **15.68 (yolow64)** |
| 128 B | 17.40 | 13.74 (Turbo-Base64) | **17.18** | **17.18 (yolow64)** |

The short `fastest dec` column is the recorded default partial-check run; under `FULLCHECK=1` Turbo-Base64 is slower, so the yolow64 full-validation wins are not weakened.

![Short decode latency — all compared libraries](docs/figures/short-all.svg)

*Short decode latency across the eight compared libraries, ns/call; lower is better.*

Short encode is now a strength: yolow64 is fastest at 8, 12, 16, 24, 32, 48 and 96 B; Turbo-Base64 wins 64 B and 128 B. Short decode is also fastest at 12, 16, 24, 48, 96 and 128 B. The exact-length decode fast path bypasses the general dispatch for no-padding 16/32/64/128-character inputs; the 24-character case now runs a single `avx2+bmi2` function (128-bit head plus `pext` tail), and the 44- and 172-character cases use `decode32`/hash-loop heads plus `pext`. In the broader 22-way snapshot, `tape-base64` is still slightly ahead at 24 B encode (8.78 versus 9.01). The new 8/16/32-byte encode single-load paths use one 24-byte conversion and overwrite the padding characters, avoiding the generic tail. The 8 KiB–1 MiB decode path gained 5–7% from speculative validation plus the hash loop (see [`research/decode.md`](research/decode.md)).

### 64 KiB throughput, GB/s (higher is better)

| Operation | yolow64 | base64-scalar | base64-simd-engine | base64-simd | base64-turbo | encodify | simdutf | Turbo-Base64 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| encode | 22.5 | 2.5 | 10.0 | 10.7 | 20.9 | 21.4 | 18.7 | **23.7** |
| decode | **20.1** | 2.1 | 12.3 | 7.9 | 18.0 | 18.5 | 14.5 | 19.4 |

### Large throughput, GB/s (higher is better)

| Input | yolow64 enc | Turbo-Base64 enc | yolow64 dec | Turbo-Base64 dec (full) |
|---:|---:|---:|---:|---:|
| 1 MiB | 22.3 | 21.0 | 20.2 | 16.7 |
| 4 MiB | 17.7 | 16.8 | 17.1 | 13.9 |

Encode is ahead at 1 MiB and 4 MiB, close at 256 KiB, and still about 5–8% behind at 4–64 KiB. The 64 KiB decode row uses Turbo-Base64 with `FULLCHECK=1`; its default partial-check build reports 26.1 GB/s. Decode gained about 5–7% at 8 KiB–1 MiB from speculative validation plus the hash loop; it is now 20.2 GB/s at 1 MiB and 17.1 GB/s at 4 MiB. It validates every block and leads the fully validating comparison from 64 KiB through 4 MiB; Turbo-Base64 is shown with `FULLCHECK=1` for a like-for-like configuration; its default build checks only the first of four blocks in the 256-byte hot loop (25%), while `FULLCHECK=1` checks every block. At 4 MiB the two are within run-to-run noise; the table should not be read as a stable ranking at that size.

![Decode throughput — all compared libraries](docs/figures/decode-all.svg)

*Decode throughput across the eight compared libraries, GB/s; higher is better. Turbo uses FULLCHECK=1.*

![Encode throughput — recorded 22-way comparison](docs/figures/encode-all.svg)

*Recorded 22-way encode comparison, GB/s; higher is better. External columns were not re-ranked.*

After this eight-library run, encode gained an adaptive streaming-store path for buffers whose input plus output exceed the detected last-level cache, plus three-line prefetch coverage per 192-byte loop iteration. A targeted three-process run gives **22.4 GB/s at 1 MiB, 21.7 GB/s at 4 MiB and 13.0 GB/s at 16 MiB** (13.0–13.3 across campaigns), up from 7.36 GB/s at 16 MiB (1.77x), and above the recorded 10.96 GB/s for `tape-base64` and 10.70 GB/s for `base64-turbo`. The threshold is `3/7` of the CPU's detected LLC, so it scales with the processor; unknown cache information or an unaligned output falls back to cached stores. See [`research/encode-counters.md`](research/encode-counters.md).

### CRLF-tolerant decode, GB/s (higher is better)

| Input | yolow64 (native) | encodify (native) | simdutf (native) | Turbo-Base64 (filter+strict) |
|---:|---:|---:|---:|---:|
| 4 KiB | 7.3 | **9.9** | 7.9 | 1.2 |
| 64 KiB | 9.2 | **10.3** | 8.1 | 1.1 |
| 1 MiB | 8.5 | **9.7** | 7.8 | 1.1 |
| 4 MiB | 6.2 | **7.3** | 6.2 | 1.0 |

The 76-column MIME fast path closed most of the previous CRLF gap: the previous implementation was about 2× slower than encodify at 1 MiB, and is now roughly 10% behind it. The libraries' accepted inputs and error semantics differ; `filter+strict` includes the whitespace-filtering time.

### Owned, append, streaming and parallel APIs

- `encode_to_string`, `append_encode`, `decode_to_vec` and `append_decode` now write into spare `Vec` capacity instead of zero-filling it first.
- `Decoder::feed` decodes whole groups in bulk and only falls back to the byte-wise path for padding or whitespace. A 1 MiB strict message fed in 4 KiB chunks improved from about 0.14 GB/s in the old byte-wise implementation to about 9 GB/s.
- `encode_streaming` is the one-shot encode entry point. From 4 KiB upward it uses 16-byte non-temporal stores when the output is aligned, avoiding the output read-for-ownership. In a cold-buffer benchmark it improves 4 KiB–4 MiB encode by 1.39–1.68x over the cache-friendly `encode`; the default remains `encode` for reused or immediately reread buffers.
- `encode_to_writer` keeps peak library memory bounded: it encodes through a fixed 64 KiB buffer. For a 256 MiB input, measured peak RSS is about 258 MiB versus 599 MiB for `encode_to_string`.
- The optional `parallel` feature adds cache-aware `encode_parallel` / `decode_parallel`. On this i7-10700, four workers improve encode by roughly 20–25% from 4 to 6.5 MiB; at 7 MiB and above the single-thread streaming-store path is faster, so the automatic policy keeps those inputs single-threaded. Decode still uses the conservative two-worker policy until its own crossover round. See [`benchmarks/README.md`](benchmarks/README.md), [`research/encode-counters.md`](research/encode-counters.md) and [`docs/API.md`](docs/API.md).

## Features and guarantees

- No allocation is required for the caller-provided buffer encode/decode APIs. Owned and append APIs allocate as needed.
- Runtime detection is cached. AVX2 and BMI2 are detected independently; the BMI2 `pext` short-decode tails run only when both are present, and AVX2-only or scalar CPUs fall back without executing `pext`. Short strict decode also uses exact-length SIMD fast paths for no-padding 16/32/64/128-character inputs. The Linux x86-64 AVX2 decoder uses an assembly bulk loop; other supported targets use their available SIMD or scalar path. The optional parallel feature is feature-gated and never spawns threads from the default APIs.
- Differential tests compare the four alphabet/padding combinations against `base64`; boundary, malformed-input, whitespace, streaming, owned/append, parallel and buffer tests are included.

Run `cargo test --release --locked` to test the library. See the [API summary](docs/API.md) and the Rust documentation generated by `cargo doc --no-deps`.

## License

[MIT](LICENSE). Third-party attribution and notices: [THIRD-PARTY.md](THIRD-PARTY.md).
