use lz4_compression::LZ4FrameReader;
use std::fs::File;
use std::io::{self, BufRead, Write};
use std::env;

fn main() -> io::Result<()> {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let file_in = File::open(filename_in)?;
    let mut file_out = File::create(filename_out)?;


    let mut lz4_reader = LZ4FrameReader::new(file_in)?.into_read();
    loop {
    	let buf = lz4_reader.fill_buf()?;
    	if buf.is_empty() { break; }
    	let consumed = file_out.write(buf)?;
    	drop(buf);
        lz4_reader.consume(consumed);
    }

    /*
    This is more convenient, but slower as io::copy does not take advantage of BufRead (i.e. we copy through one more buffer).
    let mut buf_writer = BufWriter::with_capacity(32 * 1024, file_out); // need this because io::copy only uses 8K buffers
    io::copy(&mut lz4_reader, &mut buf_writer)?;
    */

    Ok(())
}
