use lz4_compression::LZ4FrameReader;
use std::fs::File;
use std::io::{self, BufWriter, BufReader};
use std::env;

fn main() -> io::Result<()> {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let file_in = File::open(filename_in)?;
    let mut file_out = File::create(filename_out)?;
    let mut lz4_reader = LZ4FrameReader::new(file_in)?.into_read();
    let mut buf_writer = BufWriter::with_capacity(32 * 1024, file_out);
    io::copy(&mut lz4_reader, &mut buf_writer)?;

    Ok(())
}
