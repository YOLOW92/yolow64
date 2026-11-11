//! Peak-RSS comparison for owned versus writer encode.
//!
//! ```sh
//! cargo build --release --manifest-path benchmarks/compare/Cargo.toml \
//!   --bin encode_memory
//! /usr/bin/time -f 'max_rss_kb=%M' \
//!   benchmarks/compare/target/release/encode_memory string 256
//! /usr/bin/time -f 'max_rss_kb=%M' \
//!   benchmarks/compare/target/release/encode_memory writer 256
//! ```

use std::hint::black_box;

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().expect("string|writer");
    let mib: usize = args.next().expect("MiB").parse().unwrap();
    let size = mib * 1024 * 1024;
    let input: Vec<u8> = (0..size).map(|i| (i * 73 + 19) as u8).collect();

    match mode.as_str() {
        "string" => {
            let encoded = yolow64::STANDARD.encode_to_string(&input).unwrap();
            black_box(encoded.len());
        }
        "writer" => {
            let mut sink = std::io::sink();
            yolow64::STANDARD
                .encode_to_writer(&input, &mut sink)
                .unwrap();
        }
        other => panic!("unknown mode: {other}"),
    }
}
