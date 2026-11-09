use base64::{Engine, engine::general_purpose::STANDARD};
use base64_simd::AsOut;
use std::hint::black_box;
use std::time::Instant;

fn aligned(n: usize) -> (Vec<u8>, usize) {
    let v = vec![0; n + 64];
    let start = (64 - v.as_ptr() as usize % 64) % 64;
    (v, start)
}
fn measure(mut f: impl FnMut(), iters: usize) -> f64 {
    let start = Instant::now();
    for _ in 0..iters { f(); }
    start.elapsed().as_secs_f64() * 1e6 / iters as f64
}
fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}
fn main() {
    println!("size,mode,rounds,ours_enc,base64_enc,simd_enc,ours_dec,base64_dec,simd_dec");
    for size in [8usize,16,24,32,48,63,64,96,128,4096,65536,1048576,4194304] {
        for whitespace in [false, true] {
            let (mut input_mem, input_start) = aligned(size);
            for i in 0..size { input_mem[input_start+i] = (i * 73 + 19) as u8; }
            let input = &input_mem[input_start..input_start+size];
            let encoded = STANDARD.encode(input);
            let mut raw = Vec::new();
            for chunk in encoded.as_bytes().chunks(76) {
                raw.extend_from_slice(chunk);
                if whitespace { raw.extend_from_slice(b"\r\n"); }
            }
            let (mut wire_mem, wire_start) = aligned(raw.len());
            wire_mem[wire_start..wire_start+raw.len()].copy_from_slice(&raw);
            let wire = &wire_mem[wire_start..wire_start+raw.len()];
            let capacity = encoded.len().max(wire.len()).max(size);
            let (mut out_mem, out_start) = aligned(capacity);
            let output = &mut out_mem[out_start..out_start+capacity];
            let (mut filtered_mem, filtered_start) = aligned(wire.len());
            let filtered = &mut filtered_mem[filtered_start..filtered_start+wire.len()];
            let codec = yolow64::Codec {ignore_whitespace: whitespace, ..yolow64::STANDARD};
            assert_eq!(codec.encode(input, output).unwrap(), encoded.len());
            assert_eq!(&output[..encoded.len()], encoded.as_bytes());
            assert_eq!(base64_simd::STANDARD.encode(input, output.as_out()), encoded.as_bytes());
            let result = if whitespace {base64_simd::forgiving_decode(wire, output.as_out()).unwrap()} else {base64_simd::STANDARD.decode(wire, output.as_out()).unwrap()};
            assert_eq!(result, input);
            assert_eq!(codec.decode(wire, output).unwrap(), size);
            assert_eq!(&output[..size], input);
            let iters = (16 * 1024 * 1024 / size).clamp(32, 500_000);
            let mut values = [[0f64;7];6];
            for round in 0..7 {
                let mut run = |which| match which {
                    0 => { codec.encode(black_box(input), black_box(&mut *output)).unwrap(); },
                    1 => { STANDARD.encode_slice(black_box(input), black_box(&mut *output)).unwrap(); },
                    2 => { black_box(base64_simd::STANDARD.encode(black_box(input), black_box(&mut *output).as_out())); },
                    3 => { codec.decode(black_box(wire), black_box(&mut *output)).unwrap(); },
                    4 => {
                        let source = if whitespace {
                            let mut len = 0;
                            for &b in black_box(wire) {
                                if !matches!(b,b' ' | b'\t' | b'\r' | b'\n') { filtered[len]=b; len+=1; }
                            }
                            &filtered[..len]
                        } else { wire };
                        STANDARD.decode_slice(black_box(source), black_box(&mut *output)).unwrap();
                    },
                    _ => { if whitespace { black_box(base64_simd::forgiving_decode(black_box(wire), black_box(&mut *output).as_out()).unwrap()); } else { black_box(base64_simd::STANDARD.decode(black_box(wire), black_box(&mut *output).as_out()).unwrap()); } },
                };
                for step in 0..6 {
                    let which = if round % 2 == 0 { (step+round)%6 } else { (round+6-step)%6 };
                    values[which][round] = measure(||run(which),iters);
                }
            }
            let result: Vec<f64> = values.iter_mut().map(|v|median(v)).collect();
            println!("{size},{},7,{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}",if whitespace {"crlf"} else {"strict"},result[0],result[1],result[2],result[3],result[4],result[5]);
        }
    }
}
