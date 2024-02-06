use lz_fear::framed::CompressionSettings;
use std::fs::File;
use std::{io, env};
use culpa::throws;

#[throws(io::Error)]
fn main() {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let file_in = File::open(filename_in)?;
    let file_out = File::create(filename_out)?;
    
    CompressionSettings::default()
        .content_checksum(true)
        .independent_blocks(true)
        /*.block_size(64 * 1024).dictionary(0, &vec![0u8; 64 * 1024]).dictionary_id_nonsense_override(Some(42))*/
        .compress_with_size(file_in, file_out)?;
}
