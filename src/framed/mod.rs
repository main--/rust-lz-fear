//! The LZ4 frame format.
//!
//! An lz4-compressed file typically consists of a single frame.
//!
//! The frame format is self-terminating, i.e. it can be embedded without a length prefix.
//! This also allows LZ4 frames to be concatenated back to back.
//!
//! See `CompressionSettings` for the features and flexibility that the format offers.


mod compress;
mod decompress;
mod header;

/// The four magic bytes at the start of every LZ4 frame (little endian).
pub const MAGIC: u32 = 0x184D2204;
/// The frame format sets the high bit of every length field to indicate that the data was not compressed.
const INCOMPRESSIBLE: u32 = 1 << 31;
/// The LZ4 raw format maintains a lookback window of exactly 64KiB.
pub const WINDOW_SIZE: usize = 64 * 1024;


pub use compress::*;
pub use decompress::*;

