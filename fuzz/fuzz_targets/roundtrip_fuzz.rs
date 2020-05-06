#![no_main]
use libfuzzer_sys::fuzz_target;
use lz_fear::framed::{CompressionSettings, LZ4FrameReader};
use std::io::{Cursor, Read};

fuzz_target!(|data: &[u8]| {
    let input = Cursor::new(data);
    let mut output = Vec::new();

    CompressionSettings::default()
        .content_checksum(true)
        .independent_blocks(true)
        .compress(input, &mut output) //_with_size(input, &mut output)
        .expect("Could not compress input data");

    let mut lz4_reader = LZ4FrameReader::new(Cursor::new(output))
        .expect("Could not create frame reader")
        .into_read();

    let mut roundtripped = Vec::new();
    lz4_reader.read_to_end(&mut roundtripped).expect("Could not read decompressed data");
    assert!(roundtripped.iter().eq(data));
});
