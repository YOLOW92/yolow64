#![cfg(feature = "parallel")]

use yolow64::{Alphabet, Codec, Error, STANDARD};

fn data(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 73 + 19) as u8).collect()
}

fn codecs() -> [Codec; 4] {
    [
        STANDARD,
        Codec {
            alphabet: Alphabet::UrlSafe,
            padding: true,
            ignore_whitespace: false,
        },
        Codec {
            alphabet: Alphabet::Standard,
            padding: false,
            ignore_whitespace: false,
        },
        Codec {
            alphabet: Alphabet::UrlSafe,
            padding: false,
            ignore_whitespace: false,
        },
    ]
}

#[test]
fn parallel_encode_matches_single_thread() {
    let lens = [
        0,
        1,
        2,
        3,
        4,
        5,
        23,
        24,
        25,
        31,
        32,
        33,
        95,
        96,
        97,
        8 * 1024 * 1024 - 1,
        8 * 1024 * 1024,
        8 * 1024 * 1024 + 1,
        8 * 1024 * 1024 + 2,
        9 * 1024 * 1024 + 5,
        12 * 1024 * 1024,
        16 * 1024 * 1024,
        16 * 1024 * 1024 + 1,
    ];
    for codec in codecs() {
        for &len in &lens {
            let input = data(len);
            let expected_len = codec.encoded_len(len).unwrap();
            let mut expected = vec![0u8; expected_len];
            let n = codec.encode(&input, &mut expected).unwrap();
            assert_eq!(n, expected_len);

            for threads in [1, 2, 4, 8] {
                let mut output = vec![0u8; expected_len];
                let n = codec
                    .encode_parallel_with_threads(&input, &mut output, threads)
                    .unwrap();
                assert_eq!(n, expected_len, "len={len} threads={threads}");
                assert_eq!(output, expected, "len={len} threads={threads}");
            }
        }
    }
}

#[test]
fn parallel_decode_matches_single_thread() {
    let lens = [
        3,
        4,
        5,
        15,
        31,
        32,
        33,
        95,
        96,
        97,
        6 * 1024 * 1024 - 1,
        6 * 1024 * 1024,
        6 * 1024 * 1024 + 1,
    ];
    for codec in codecs() {
        for &len in &lens {
            let input = data(len);
            let encoded = codec.encode_to_string(&input).unwrap();
            let decoded_len = codec.validate(encoded.as_bytes()).unwrap();
            let mut expected = vec![0u8; decoded_len];
            let n = codec.decode(encoded.as_bytes(), &mut expected).unwrap();
            assert_eq!(n, decoded_len);

            for threads in [1, 2, 4, 8] {
                let mut output = vec![0u8; decoded_len];
                let n = codec
                    .decode_parallel_with_threads(encoded.as_bytes(), &mut output, threads)
                    .unwrap();
                assert_eq!(n, decoded_len, "len={len} threads={threads}");
                assert_eq!(output, expected, "len={len} threads={threads}");
            }
        }
    }
}

#[test]
fn parallel_whitespace_falls_back() {
    let input = data(9 * 1024 * 1024);
    let encoded = STANDARD.encode_to_string(&input).unwrap();
    let mut wire = Vec::new();
    for chunk in encoded.as_bytes().chunks(76) {
        wire.extend_from_slice(chunk);
        wire.extend_from_slice(b"\r\n");
    }
    let codec = Codec {
        ignore_whitespace: true,
        ..STANDARD
    };
    let mut expected = vec![0u8; input.len()];
    let n = codec.decode(&wire, &mut expected).unwrap();
    assert_eq!(n, input.len());
    for threads in [1, 2, 8] {
        let mut output = vec![0u8; input.len()];
        let n = codec
            .decode_parallel_with_threads(&wire, &mut output, threads)
            .unwrap();
        assert_eq!(n, input.len());
        assert_eq!(output, expected);
    }
}

#[test]
fn parallel_errors_match_single_thread() {
    let input = data(9 * 1024 * 1024);
    let encoded = STANDARD.encode_to_string(&input).unwrap();
    for pos in [
        0,
        1,
        encoded.len() / 3,
        encoded.len() / 2,
        encoded.len() * 2 / 3,
        encoded.len() - 2,
        encoded.len() - 1,
    ] {
        let mut bad = encoded.clone().into_bytes();
        bad[pos] = b'!';
        let mut scratch = vec![0u8; input.len() + 8];
        let single = STANDARD.decode(&bad, &mut scratch).unwrap_err();
        assert!(matches!(single, Error::InvalidByte { .. }));
        for threads in [1, 2, 4, 8] {
            let mut output = vec![0u8; input.len() + 8];
            let parallel = STANDARD
                .decode_parallel_with_threads(&bad, &mut output, threads)
                .unwrap_err();
            assert_eq!(parallel, single, "pos={pos} threads={threads}");
        }
    }
}

#[test]
fn parallel_output_too_small() {
    let input = data(8 * 1024 * 1024);
    let encoded_len = STANDARD.encoded_len(input.len()).unwrap();
    let mut output = vec![0u8; encoded_len - 1];
    assert_eq!(
        STANDARD.encode_parallel(&input, &mut output),
        Err(Error::OutputTooSmall)
    );

    let encoded = STANDARD.encode_to_string(&input).unwrap();
    let mut decoded = vec![0u8; input.len() - 1];
    assert_eq!(
        STANDARD.decode_parallel(encoded.as_bytes(), &mut decoded),
        Err(Error::OutputTooSmall)
    );
}

#[test]
fn parallel_rejects_trailing_unpadded_and_malformed_lengths() {
    for alphabet in [Alphabet::Standard, Alphabet::UrlSafe] {
        for padding in [true, false] {
            let codec = Codec {
                alphabet,
                padding,
                ignore_whitespace: false,
            };
            for base_quads in [2usize, 8, 64] {
                let prefix = vec![b'A'; base_quads * 4];
                for tail in [
                    &b"T"[..],
                    &b"TQ"[..],
                    &b"TWE"[..],
                    &b"TQ="[..],
                    &b"T="[..],
                    &b"=="[..],
                    &b"A!"[..],
                ] {
                    let mut sample = prefix.clone();
                    sample.extend_from_slice(tail);
                    let mut single_out = vec![0xa5u8; sample.len()];
                    let single = codec.decode(&sample, &mut single_out);
                    for threads in [1, 2, 4] {
                        let mut parallel_out = vec![0xa5u8; sample.len()];
                        let parallel =
                            codec.decode_parallel_with_threads(&sample, &mut parallel_out, threads);
                        assert_eq!(
                            parallel, single,
                            "codec={codec:?} base_quads={base_quads} tail={tail:?} threads={threads}"
                        );
                        if let Ok(n) = parallel {
                            assert_eq!(&parallel_out[..n], &single_out[..n]);
                            assert!(parallel_out[n..].iter().all(|&b| b == 0xa5));
                        }
                    }
                }
            }
        }
    }
}
