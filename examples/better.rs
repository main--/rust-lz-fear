use lz4_compression::compress::{compress2,U32Table};
use std::fs::File;
use std::io::{self, Write, Read};
use std::env;

use byteorder::{WriteBytesExt, LE};

fn main() -> io::Result<()> {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let mut file_in = File::open(filename_in)?;
    let mut file_out = File::create(filename_out)?;

    let mut buf = Vec::new();
    file_in.read_to_end(&mut buf)?;

    file_out.write_u32::<LE>(0x184D2204)?;
    file_out.write_u8(0x60)?;
    file_out.write_u8(0x70)?;
    file_out.write_u8(0x73)?;
    


    let mut compressed2 = Vec::with_capacity(5 * 1024 * 1024);
    let mut source = buf.as_slice();
    for chunk in source.chunks(4*1024*1024) {
    compress2::<_, U32Table>(chunk, &mut compressed2)?;
    file_out.write_u32::<LE>(compressed2.len() as u32)?;
    file_out.write_all(&compressed2)?;
    compressed2.clear();
    }
    file_out.write_u32::<LE>(0)?;

//    assert!(compressed.iter().eq(compressed2.iter()));

//    file_out.write_all(&compressed2)?;
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
