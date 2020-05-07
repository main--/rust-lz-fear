#![no_main]
use libfuzzer_sys::fuzz_target;
use lz_fear::framed::{CompressionSettings, LZ4FrameReader};
use std::io::{Cursor, Read};

fuzz_target!(|data: &[u8]| {
    let input = Cursor::new(data);
    let mut output = Vec::new();
    let lz4_reader = LZ4FrameReader::new(input);
    if let Ok(reader) = lz4_reader {
        // we deliberately ignore errors here because random bytes from fuzzer
        // are not valid LZ4 data and so are expected to trigger non-fatal errors
        let _ = reader.into_read().read_to_end(&mut output);
    }
});
