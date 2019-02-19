[![Crate](https://img.shields.io/crates/v/lz4-compression.svg)](https://crates.io/crates/lz4-compression)
[![Documentation](https://docs.rs/lz4-compression/badge.svg)](https://docs.rs/crate/lz4-compression/)


# lz4-compression

Pure rust implementation of lz4 compression and decompression.

This is based on [redox-os' lz4 compression](https://github.com/redox-os/tfs/tree/master/lz4),
but has been reworked to be actually usable as a library crate.

(The [redox-os lz4 crate](https://crates.io/crates/lz4-compress) 
does not re-export the error types 
and does not use standard IO Writers and Readers.)

Usage: 
```rust
use lz4_compression::prelude::{ decompress, compress };

fn main(){
    let uncompressed_data: &[u8] = b"Hello world, what's up?";

    let compressed_data = compress(uncompressed_data);
    let decompressed_data = decompress(&compressed_data).unwrap();

    assert_eq!(uncompressed_data, decompressed_data.as_slice());
}
```