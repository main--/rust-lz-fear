use lz4_compression::compress::compress;
use lz4_compression::decompress::decompress;
use rand::prelude::*;
use std::time::Instant;

fn main(){
    let mut data = vec![0u8; 10_000_000];
    thread_rng().fill(&mut data[2_000_000..6_000_000]); // mixed

    let uncompressed_data: &[u8] = data.as_slice();
    let compressed_data = compress(uncompressed_data);

    {
	let pre = Instant::now();
	for _ in 0..100 {
            let decompressed_data = decompress(&compressed_data).unwrap();
//            assert_eq!(uncompressed_data, decompressed_data.as_slice());
	}
	let duration = pre.elapsed();
	println!("{:?}", duration);
    }

/*
    {
	let pre = Instant::now();
	for _ in 0..100 {
            let decompressed_data = decompress3(&compressed_data).unwrap();
//            assert_eq!(uncompressed_data, decompressed_data.as_slice());
	}
	let duration = pre.elapsed();
	println!("{:?}", duration);
    }
*/
}
