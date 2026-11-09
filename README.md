# yolow64

[中文](docs/README.zh-CN.md)

A Base64 codec for Rust, with a small API and runtime-dispatched SIMD. It uses only the Rust standard library at runtime.

## Story

While building [PageSift](https://github.com/YOLOW92/pagesift), we needed Base64 for browser screenshots, MCP resources and proxy authentication. Those call sites made correctness, allocation control and real-world throughput equally important. yolow64 extracts that codec into an independent MIT-licensed library.

## At a glance

- Standard and URL-safe alphabets; padded and unpadded forms.
- Slice, append, owned-output, validation, streaming decode and data URL APIs.
- AVX2 on supported x86-64 CPUs and NEON on supported AArch64 CPUs, selected at runtime; scalar fallback.
- Rust 1.89 or newer; `std` required. **No runtime third-party dependencies.**
- Independently measured results and runnable benchmark below. Short inputs do not always win.

## Install

Use the Git repository as a Cargo dependency (the crate has not been published to crates.io):

```toml
[dependencies]
yolow64 = { git = "https://github.com/YOLOW92/yolow64" }
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

Microseconds per call, **lower is better**. Intel Core i7-10700, Linux x86-64, CPU 0, Rust 1.99.0, release with fat LTO. Three complete same-process runs; each cell is the median of the three runs' seven-round medians. Inputs and reused output buffers are 64-byte aligned. The input is repeatedly reused, so results do not describe cold-cache workloads or all buffer alignments. Short-input differences are in nanoseconds and sensitive to noise.

The comparison uses `base64` 0.23.1 and `base64-simd` 0.8.0. For CRLF decoding, `base64` timing includes filtering whitespace into a reused buffer; `base64-simd` uses `forgiving_decode`. Their full acceptance and error semantics differ from yolow64. All libraries decode the valid padded test inputs correctly. Encoding is unaffected by whitespace mode; differences between its two tables are measurement noise. [Method, commands and raw results](benchmarks/README.md).

### Strict

| Input bytes | yolow64 enc | base64 enc | base64-simd enc | yolow64 dec | base64 dec | base64-simd dec |
|---:|---:|---:|---:|---:|---:|---:|
| 8 | 0.0110 | 0.0094 | 0.0073 | 0.0127 | 0.0139 | 0.0082 |
| 16 | 0.0162 | 0.0150 | 0.0090 | 0.0196 | 0.0195 | 0.0119 |
| 24 | 0.0114 | 0.0183 | 0.0112 | 0.0147 | 0.0246 | 0.0149 |
| 32 | 0.0142 | 0.0189 | 0.0137 | 0.0231 | 0.0251 | 0.0105 |
| 48 | 0.0128 | 0.0277 | 0.0133 | 0.0160 | 0.0357 | 0.0198 |
| 63 | 0.0136 | 0.0316 | 0.0142 | 0.0303 | 0.0415 | 0.0167 |
| 64 | 0.0153 | 0.0329 | 0.0149 | 0.0317 | 0.0423 | 0.0187 |
| 96 | 0.0149 | 0.0466 | 0.0172 | 0.0213 | 0.0598 | 0.0253 |
| 128 | 0.0183 | 0.0563 | 0.0222 | 0.0302 | 0.0729 | 0.0224 |
| 4096 | 0.1919 | 1.6115 | 0.3794 | 0.2604 | 1.9843 | 0.5265 |
| 65536 | 3.1836 | 25.8625 | 6.1087 | 3.6381 | 31.7227 | 8.2179 |
| 1048576 | 57.7037 | 416.5615 | 103.2870 | 58.7227 | 513.3446 | 135.5395 |
| 4194304 | 269.9127 | 1730.8464 | 473.7456 | 266.8989 | 2148.9401 | 605.4132 |

### CRLF input

| Input bytes | yolow64 enc | base64 enc | base64-simd enc | yolow64 dec | base64 dec | base64-simd dec |
|---:|---:|---:|---:|---:|---:|---:|
| 8 | 0.0110 | 0.0094 | 0.0072 | 0.0163 | 0.0213 | 0.0175 |
| 16 | 0.0161 | 0.0143 | 0.0075 | 0.0231 | 0.0332 | 0.0230 |
| 24 | 0.0114 | 0.0183 | 0.0113 | 0.0179 | 0.0426 | 0.0225 |
| 32 | 0.0142 | 0.0189 | 0.0137 | 0.0272 | 0.0490 | 0.0259 |
| 48 | 0.0128 | 0.0277 | 0.0132 | 0.0243 | 0.0791 | 0.0265 |
| 63 | 0.0136 | 0.0316 | 0.0142 | 0.0559 | 0.0893 | 0.0356 |
| 64 | 0.0153 | 0.0329 | 0.0149 | 0.0619 | 0.0893 | 0.0389 |
| 96 | 0.0149 | 0.0466 | 0.0172 | 0.0385 | 0.1272 | 0.0646 |
| 128 | 0.0184 | 0.0564 | 0.0222 | 0.0693 | 0.1714 | 0.0888 |
| 4096 | 0.1920 | 1.6127 | 0.3832 | 0.9862 | 5.4392 | 3.9881 |
| 65536 | 3.1859 | 25.8590 | 6.1024 | 15.4603 | 87.0181 | 63.7618 |
| 1048576 | 57.3887 | 417.8530 | 102.6008 | 254.0882 | 1409.5056 | 1041.0268 |
| 4194304 | 264.4627 | 1763.5911 | 470.4936 | 1107.6406 | 5968.3779 | 4424.0290 |

## Features and guarantees

- No allocation is required for the caller-provided buffer encode/decode APIs. Owned and append APIs allocate as needed.
- Runtime detection is cached; short inputs use scalar paths. The Linux x86-64 AVX2 decoder uses an assembly bulk loop; other supported targets use their available SIMD or scalar path.
- Differential tests compare the four alphabet/padding combinations against `base64`; boundary, malformed-input, whitespace, streaming and buffer tests are included.

Run `cargo test --release --locked` to test the library. See the [API summary](docs/API.md) and the Rust documentation generated by `cargo doc --no-deps`.

## License

[MIT](LICENSE). Third-party attribution and notices: [THIRD-PARTY.md](THIRD-PARTY.md).
