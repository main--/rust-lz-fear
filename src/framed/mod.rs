mod compress;
mod decompress;
mod header;

/// The four magic bytes at the start of every LZ4 frame.
const MAGIC: u32 = 0x184D2204;
/// The frame format sets the high bit of every length field to indicate that the data was not compressed.
const INCOMPRESSIBLE: u32 = 1 << 31;
/// The LZ4 raw format maintains a lookback window of exactly 64KiB.
const WINDOW_SIZE: usize = 64 * 1024;


pub use compress::*;
pub use decompress::*;

