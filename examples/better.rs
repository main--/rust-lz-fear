use lz4_compression::decompress_file;
use std::io::Cursor;

fn main(){
    assert!(decompress_file(Cursor::new(include_bytes!("../smol.bin") as &[u8])).iter().cloned().eq(vec![0u8; 100 * 1024 * 1024]));
    assert!(decompress_file(Cursor::new(include_bytes!("../random.bin.lz4") as &[u8])).iter().eq(include_bytes!("../random.bin").iter()));
    assert!(decompress_file(Cursor::new(include_bytes!("../sh.lz4") as &[u8])).iter().eq(include_bytes!("/usr/bin/cargo").iter()));
}
