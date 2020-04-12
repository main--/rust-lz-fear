use lz4_compression::compress::compress;
use std::fs::File;
use std::io::{self, Write, Read};
use std::env;

fn main() -> io::Result<()> {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let mut file_in = File::open(filename_in)?;
    let mut file_out = File::create(filename_out)?;

    let mut buf = Vec::new();
    file_in.read_to_end(&mut buf)?;

    let compressed = compress(&buf);
    file_out.write_all(&compressed)?;
//    assert!(lz4_compression::decompress::decompress(&compressed).unwrap().iter().copied().eq(buf));

/*
let mut buf = Vec::with_capacity(4 * 1024 * 1024);
while file_in.read(&mut buf)? > 0 {
    compress(&buf);
    buf.clear();
}
*/

    Ok(())
}
