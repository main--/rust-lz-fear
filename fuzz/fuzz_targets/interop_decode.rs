#![no_main]
use libfuzzer_sys::fuzz_target;
use lz_fear::framed::{CompressionSettings, LZ4FrameReader};
use std::io::{Cursor, Read};

fuzz_target!(|data: &[u8]| {
    let compression_result = reference_compress(data);
    if let Ok(compressed) = compression_result {
        let input = Cursor::new(compressed);
        let mut decompressed = Vec::new();
        let mut lz4_reader = LZ4FrameReader::new(input).expect("Failed to create reader").into_read();
        lz4_reader.read_to_end(&mut decompressed).expect("Failed to decompress data compressed by C implementation");
        assert!(data == decompressed.as_slice(), "Decompression result did not match the original input");
    }
});

// compress data using the reference lz4 implementation 
fn reference_compress(data: &[u8]) -> Result<Vec<u8>,()> {
    let mut input = std::io::Cursor::new(data);
    let output = std::io::Cursor::new(Vec::new());
    let mut encoder = lz4::EncoderBuilder::new()
        .level(4)
        .build(output).unwrap();
    std::io::copy(&mut input, &mut encoder).unwrap();
    let (output, result) = encoder.finish();
    if result.is_ok() {
        Ok(output.into_inner())
    } else {
        Err(())
    }
}
