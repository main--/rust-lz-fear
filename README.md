# lz4-compression

Pure rust implementation of lz4 compression and decompression.

This is based on [redox-os' lz4 compression](https://github.com/redox-os/tfs/tree/master/lz4),
but has been reworked to be actually usable as a library crate.

(The [redox-os lz4 crate](https://crates.io/crates/lz4-compress) does not re-export the error types 
and does not use standard IO Writers and Readers.)