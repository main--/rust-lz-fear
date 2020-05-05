//! The raw LZ4 compression format.
//!
//! Using this directly saves you the overhead of framing (~11 bytes) but you lose several features,
//! most notably the fallback mechanism for incompressible data: if the compressed version of a block
//! would be larger, it encodes the uncompressed version instead. This guarantees that the compression ratio will
//! never be negative. The break-even point where framing is always smaller is around 2.5KB for totally
//! incompressible data. Conversely, for payloads below 2.5KB framing always adds a bit of overhead
//! (but also gets you lots of nice features).

mod compress;
mod decompress;

pub use compress::*;
pub use decompress::*;

