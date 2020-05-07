#![no_main]
use libfuzzer_sys::fuzz_target;
use lz_fear::framed::{CompressionSettings, LZ4FrameReader};
use std::io::{Cursor, Read};

fuzz_target!(|data: &[u8]| {
    let input = Cursor::new(data);
    let lz4_reader = LZ4FrameReader::new(input);
    if let Ok(reader) = lz4_reader {
        let mut lz4_reader = reader.into_read();
        let mut buffer = vec![0; 4096];
        let mut result = lz4_reader.read(&mut buffer);
        while result.is_ok() && result.unwrap() > 0 {
            result = lz4_reader.read(&mut buffer);
        }
    }
});
