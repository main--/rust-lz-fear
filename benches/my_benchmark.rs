//use lz4_compression::prelude::{decompress, compress};
//use lz4_compression::decompress::decompress3;
use lz4_compression::decompress::decompress;
use lz4_compression::compress::compress;
use lz4_compression::decompress_file;
use rand::prelude::*;
use criterion::{black_box, criterion_group, criterion_main, Criterion};

use std::io::Cursor;

fn criterion_benchmark(c: &mut Criterion) {
    let mut data = vec![0u8; 10_000_000];
    thread_rng().fill(&mut data[2_000_000..6_000_000]); // mixed

    let uncompressed_data: &[u8] = data.as_slice();
    let compressed_data = compress(uncompressed_data);

    let cargo = include_bytes!("../sh.lz4") as &[u8];
    c.bench_function("lz4 -d cargo", |b| b.iter(|| decompress_file(Cursor::new(cargo))));

    let mut group = c.benchmark_group("decompress");
    group.bench_with_input("ours", &compressed_data.as_slice(), |b, c| b.iter(|| decompress(c)));
    //group.bench_with_input("theirs", &compressed_data.as_slice(), |b, c| b.iter(|| decompress3(c)));
    
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
