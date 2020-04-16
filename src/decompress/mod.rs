//! LZ4 decompression.
//!
//! 

mod raw;
mod framed;

pub use raw::*;
pub use framed::*;
//pub use raw::decompress_block as decompress_raw_block;
//pub use framed::{DecompressionError, LZ4FrameReader};

