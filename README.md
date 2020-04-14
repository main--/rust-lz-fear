# lz-fear

This crate aims to implement the LZ4 compression and decompression algorithm, as well as the framing format used for LZ4 files, in **pure Rust** with **no unsafe code**.
At the time of writing, this is the *fastest* no-unsafe implementation that I'm aware of.

The [lz4](https://crates.io/crates/lz4) crate calls into the C library.
The [compress](https://crates.io/crates/compress) crate has an implementation that relies on `unsafe`.
And the redox implementation (more on that below) is slower, in some cases substantially.

Decompressor status: Beta. Works well, and is blazingly fast (at least as fast as the official C implementation in my tests).

Compressor status: WIP. The example named "better.rs" should produce an lz4-compatible output file, but the compressor is still missing an entire API.
Also the frame format is only implemented in the form of a constant header right now.
Performance is good, but takes ~2-3x as long as the C implementation. The current bottleneck appears to be an abundance of range checks when writing output (~25% of cycles spent in there)
which also cause the compiler to completely trip over itself and sometimes emit a sequence of copy_from_slice calls for 1-byte and 4-byte writes to the output array. Help wanted.

If you take a look at the git history, this is strictly speaking a fork from @johannesvollmer.
He took [redox-os' lz4 compression](https://github.com/redox-os/tfs/tree/master/lz4), and reworked it to be usable as a library crate.

However after noticing performance issues, I have gradually rewritten both the compressor and the decompressor from scratch to closely resemble the 2400-line *mess* that is the official
C implementation (and that's just the raw format without framing!). Admittedly they pack plenty of optimizations in there (lots of intentionally reading beyond buffer boundaries for the sake of performance),
but I'm proud to say that I achieved similar performance in just 400 lines of competely safe Rust.

