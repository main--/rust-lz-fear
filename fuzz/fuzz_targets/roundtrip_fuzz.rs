#![no_main]
use libfuzzer_sys::fuzz_target;
use lz_fear::framed::{CompressionSettings, LZ4FrameReader};
use std::io::Cursor;

fuzz_target!(|data: &[u8]| {
    // TODO - What's the minimum input size for lz4?
    if data.len() >= 31 {
        let input = Cursor::new(data);
        let mut output = Vec::new();

        CompressionSettings::default()
            .content_checksum(true)
            .independent_blocks(true)
            .compress_with_size(input, &mut output)
            .expect("Could not compress input data");

        let mut lz4_reader = LZ4FrameReader::new(Cursor::new(output))
            .expect("Could not create frame reader")
            .into_read();

        // TODO - Completely decompress and compare decompressed output to initial fuzz input
    }
});
