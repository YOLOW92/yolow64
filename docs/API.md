# API reference

The generated Rust documentation (`cargo doc --no-deps --open`) is the authoritative API reference. This page summarizes the entry points.

## Configuration

`Codec { alphabet, padding, ignore_whitespace }` selects `Alphabet::Standard` or `Alphabet::UrlSafe`. When `padding` is true, padding is required where needed; when false, `=` is forbidden. Unused bits in the last symbol must be zero.

`STANDARD` selects the standard alphabet, padded form and strict whitespace. `ignore_whitespace` permits exactly ASCII space, tab, CR and LF; all other invalid bytes remain errors.

## Buffers and owned values

| API | Result |
|---|---|
| `Codec::encoded_len(input_len)` | Exact encoded capacity, or `None` on overflow |
| `Codec::encode(input, output)` | Encoded bytes written, or an error |
| `Codec::decode(input, output)` | Decoded bytes written, or an error |
| `Codec::validate(input)` | Exact decoded length, or an error |
| `Codec::encode_to_string(input)` | New encoded `String` |
| `Codec::decode_to_vec(input)` | New decoded `Vec<u8>` |
| `Codec::append_encode(input, output)` | Append encoded bytes to a `Vec<u8>` |
| `Codec::append_decode(input, output)` | Append decoded bytes to a `Vec<u8>`; on error its original length is restored |
| `encode_data_url(mime, input)` | Standard padded data URL; MIME is not validated |
| `decode_standard_compat(input)` | Standard decode with `base64` 0.23-compatible error text (`Result<Vec<u8>, String>`) |

Input and output slices must be separate. On error, a caller-provided decode output can contain partial bytes and should be discarded. Errors are `InvalidByte { byte, offset }`, `InvalidLength`, `InvalidPadding`, or `OutputTooSmall`. An invalid-byte offset refers to the original input, including whitespace.

## Streaming

`Decoder::new(codec)` creates an incremental decoder. Call `feed(chunk, &mut output)` for each chunk and `finish(&mut output)` to validate the last group. Output appended before an error is not rolled back; discard it when the complete message must be valid. After an error, do not reuse the decoder.

Run `cargo run --example basic` for a compilable example.
