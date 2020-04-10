//! Pure Rust implementation of LZ4 compression.
//!
//! A detailed explanation of the algorithm can be found [here](http://ticki.github.io/blog/how-lz4-works/).

// TODO no-std?

pub mod decompress;
pub mod compress;

pub mod prelude {
    pub use crate::decompress::decompress;
    pub use crate::compress::compress;
}


use byteorder::{LE, ReadBytesExt};
use std::hash::Hasher;
use std::io::{Read, Write, Result as IoResult, Cursor};
use std::convert::TryInto;
use twox_hash::XxHash32;


/*
use std::ops::Index;
trait Sliceable: Index<usize> {
    fn len(&self) -> usize;
}
impl Sliceable for [u8] {
    fn len(&self) -> usize { self.len() }
}
use std::marker::PhantomData;
struct SliceConcat<T, L, R> {
    t: PhantomData<T>,
    left: L,
    right: R,
}
impl<T, L: AsRef<Sliceable<Output=T>>, R: AsRef<Sliceable<Output=T>>> Index<usize> for SliceConcat<T, L, R> {
    type Output = T;

    fn index(&self, i: usize) -> &Self::Output {
        let offset = self.left.as_ref().len();
        if i < offset {
            &self.left.as_ref()[i]
        } else {
            &self.right.as_ref()[i - offset]
        }
    }
}
impl<T, L: AsRef<Sliceable<Output=T>>, R: AsRef<Sliceable<Output=T>>> Sliceable for SliceConcat<T, L, R> {
    fn len(&self) -> usize { self.left.as_ref().len() + self.right.as_ref().len() }
}
*/



const WINDOW_SIZE: usize = 64 * 1024;


struct Flags(u8);
// TODO: debug impl
impl Flags {
    fn parse(i: u8) -> Self {
        assert_eq!(i >> 6, 0b01); // version
        assert_eq!(i & 0b10, 0); // reserved

        Flags(i)
    }

    fn independent_blocks(&self) -> bool { (self.0 & 0b00100000) != 0 }
    fn block_checksums(&self)    -> bool { (self.0 & 0b00010000) != 0 }
    fn content_size(&self)       -> bool { (self.0 & 0b00001000) != 0 }
    fn content_checksum(&self)   -> bool { (self.0 & 0b00000100) != 0 }
    fn dictionary_id(&self)      -> bool { (self.0 & 0b00000001) != 0 }
}

struct BlockDescriptor(u8); // ??? or what else could "BD" stand for ???
impl BlockDescriptor {
    fn parse(i: u8) -> Self {
        assert_eq!(i & 0b10001111, 0); // reserved bits
        BlockDescriptor(i)
    }

    fn block_maxsize(&self) -> usize {
        match (self.0 >> 4) & 0b111 {
            0 | 1 | 2 | 3 => unimplemented!("at the time of writing this, spec says these values are reserved"),
            i if i <= 7 => 1 << (i * 2 + 8),
            _ => unreachable!(),
        }
    }
}


pub struct LZ4FrameReader<R: Read> {
    reader: R,
    flags: Flags,
    bd: BlockDescriptor,
    content_size: Option<u64>,
    dictionary_id: Option<u32>,
    content_hasher: Option<XxHash32>,
    carryover_window: Option<Vec<u8>>,
}

impl<R: Read> LZ4FrameReader<R> {
    pub fn new(mut reader: R) -> IoResult<Self> {
        let magic = reader.read_u32::<LE>().unwrap();
        assert_eq!(magic, 0x184D2204);

        let flags = Flags::parse(reader.read_u8().unwrap());
        let bd = BlockDescriptor::parse(reader.read_u8().unwrap());

        let mut hasher = XxHash32::with_seed(0);
        hasher.write_u8(flags.0);
        hasher.write_u8(bd.0);

        let content_size = if flags.content_size() {
            let i = reader.read_u64::<LE>().unwrap();
            hasher.write_u64(i);
            Some(i)
        } else {
            None
        };
        println!("csiz = {:?}", content_size);
        let dictionary_id = if flags.dictionary_id() {
            let i = reader.read_u32::<LE>().unwrap();
            hasher.write_u32(i);
            Some(i)
        } else {
            None
        };

        let hc = reader.read_u8().unwrap();
        assert_eq!(hc, (hasher.finish() >> 8) as u8);
        let content_hasher = if flags.content_checksum() {
            Some(XxHash32::with_seed(0))
        } else {
            None
        };

        let carryover_window = if flags.independent_blocks() {
            None
        } else {
            Some(Vec::with_capacity(WINDOW_SIZE))
        };
        Ok(LZ4FrameReader { reader, flags, bd, content_size, dictionary_id, content_hasher, carryover_window })
    }

    pub fn block_size(&self) -> usize { self.bd.block_maxsize() }
    pub fn frame_size(&self) -> Option<u64> { self.content_size }
    pub fn dictionary_id(&self) -> Option<u32> { self.dictionary_id }

    pub fn decode_block(&mut self, output: &mut Vec<u8>) {
        assert!(output.is_empty());

        let reader = &mut self.reader;

        let block_length = reader.read_u32::<LE>().unwrap();
        if block_length == 0 {
            if let Some(hasher) = self.content_hasher.take() {
                let checksum = reader.read_u32::<LE>().unwrap();
                assert_eq!(hasher.finish(), checksum.into());
            }
            return;
        }

        let is_compressed = block_length & 0x80_00_00_00 == 0;
        let block_length = block_length & 0x7f_ff_ff_ff;

        let mut buf = vec![0u8; block_length.try_into().unwrap()];
        reader.read_exact(buf.as_mut_slice()).unwrap();

        if self.flags.block_checksums() {
            let checksum = reader.read_u32::<LE>().unwrap();
            let mut hasher = XxHash32::with_seed(0);
            hasher.write(&buf);
            assert_eq!(hasher.finish(), checksum.into());
        }

        if is_compressed {
            if let Some(window) = self.carryover_window.as_mut() {
//                let mut vec = Vec::with_capacity(self.bd.block_maxsize());
                decompress::decompress_block(&buf, &window, output).unwrap();
//                decompress::decompress_into(&buf, &mut vec).unwrap();

                let outlen = output.len();
                if outlen < WINDOW_SIZE {
                    // remove as many bytes from front as we are replacing
                    window.drain(..outlen);
                    window.extend_from_slice(&output);
                } else {
                    window.clear();
                    window.extend_from_slice(&output[outlen - WINDOW_SIZE..]);
                }

                assert!(window.len() <= WINDOW_SIZE);
println!("dependently compressed {} {}", window.capacity(), window.len());
            } else {
                decompress::decompress_block(&buf, &[], output).unwrap();
println!("independently compressed");
            }
        } else {
            output.extend_from_slice(&buf);
println!("uncompressed");
        }

        assert!(output.len() <= self.bd.block_maxsize());

        if let Some(hasher) = self.content_hasher.as_mut() {
            hasher.write(&output);
        }
    }
}


pub fn decompress_file<R: Read>(reader: R) -> Vec<u8> {
    let mut reader = LZ4FrameReader::new(reader).unwrap();

    let mut plaintext = Vec::new();

    let mut buf = Vec::with_capacity(reader.block_size());
    loop {
        reader.decode_block(&mut buf);
        let len = buf.len();
        if len == 0 { break; }
        plaintext.extend_from_slice(&buf[..len]);
        buf.clear();
    }

    plaintext
}




#[cfg(test)]
mod tests {
    use std::str;
    use crate::prelude::*;

    /// Test that the compressed string decompresses to the original string.
    fn inverse(s: &str) {
        let compressed = compress(s.as_bytes());
        println!("Compressed '{}' into {:?}", s, compressed);
        let decompressed = decompress(&compressed).unwrap();
        println!("Decompressed it into {:?}", str::from_utf8(&decompressed).unwrap());
        assert_eq!(decompressed, s.as_bytes());
    }

    #[test]
    fn shakespear() {
        inverse("to live or not to live");
        inverse("Love is a wonderful terrible thing");
        inverse("There is nothing either good or bad, but thinking makes it so.");
        inverse("I burn, I pine, I perish.");
    }

    #[test]
    fn save_the_pandas() {
        inverse("To cute to die! Save the red panda!");
        inverse("You are 60% water. Save 60% of yourself!");
        inverse("Save water, it doesn't grow on trees.");
        inverse("The panda bear has an amazing black-and-white fur.");
        inverse("The average panda eats as much as 9 to 14 kg of bamboo shoots a day.");
        inverse("The Empress Dowager Bo was buried with a panda skull in her vault");
    }

    #[test]
    fn not_compressible() {
        inverse("as6yhol.;jrew5tyuikbfewedfyjltre22459ba");
        inverse("jhflkdjshaf9p8u89ybkvjsdbfkhvg4ut08yfrr");
    }

    #[test]
    fn short() {
        inverse("ahhd");
        inverse("ahd");
        inverse("x-29");
        inverse("x");
        inverse("k");
        inverse(".");
        inverse("ajsdh");
    }

    #[test]
    fn empty_string() {
        inverse("");
    }

    #[test]
    fn nulls() {
        inverse("\0\0\0\0\0\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn compression_works() {
        let s = "The Read trait allows for reading bytes from a source. Implementors of the Read trait are called 'readers'. Readers are defined by one required method, read().";

        inverse(s);

        assert!(compress(s.as_bytes()).len() < s.len());
    }

    #[test]
    fn big_compression() {
        let mut s = Vec::with_capacity(80_000000);

        for n in 0..80_000000 {
            s.push((n as u8).wrapping_mul(0xA).wrapping_add(33) ^ 0xA2);
        }

        assert_eq!(&decompress(&compress(&s)).unwrap(), &s);
    }
}
