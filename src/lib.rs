#![forbid(unsafe_code)]
#![allow(non_upper_case_globals)]

pub mod decompress;
pub mod compress;

use byteorder::{LE, ReadBytesExt, WriteBytesExt};
use std::hash::Hasher;
use std::io::{self, Read, BufRead, Write, Seek, Error as IoError, ErrorKind};
use std::cmp;
use std::mem;
use std::convert::TryInto;
use twox_hash::XxHash32;
use thiserror::Error;
use fehler::{throw, throws};
use bitflags::bitflags;
use derive_builder::Builder;

#[derive(Error, Debug)]
pub enum DecompressionError {
    #[error("error reading from the input you gave me")]
    InputError(#[from] io::Error),
    #[error("the raw LZ4 decompression failed (data corruption?)")]
    CodecError(#[from] decompress::Error),
    #[error("at the time of writing this, spec says value {0} is reserved")]
    UnimplementedBlocksize(u8),
    #[error("wrong magic number in file header: {0:08x}")]
    WrongMagic(u32),
    #[error("file version {0} not supported")]
    UnsupportedVersion(u8),
    #[error("reserved bits in flags set")]
    ReservedFlagBitsSet,
    #[error("reserved bits in bd set")]
    ReservedBdBitsSet,
    #[error("the header checksum was invalid")]
    HeaderChecksumFail,
    #[error("a block checksum was invalid")]
    BlockChecksumFail,
    #[error("the frame checksum was invalid")]
    FrameChecksumFail,
    #[error("stream contains a compressed block with a size so large we can't even compute it (let alone fit the block in memory...)")]
    BlockLengthOverflow,
    #[error("a block decompressed to more data than allowed")]
    BlockSizeOverflow,
}
type Error = DecompressionError;
impl From<DecompressionError> for io::Error {
    fn from(e: DecompressionError) -> io::Error {
        IoError::new(ErrorKind::Other, e)
    }
}


pub struct CompressionSettings<'a> {
    independent_blocks: bool,
    block_checksums: bool,
    content_checksum: bool,
    block_size: usize,
    dictionary: Option<(u32, &'a [u8])>,
}
impl<'a> Default for CompressionSettings<'a> {
    fn default() -> Self {
        Self {
            independent_blocks: true,
            block_checksums: false,
            content_checksum: true,
            block_size: 4 * 1024 * 1024,
            dictionary: None,
        }
    }
}
impl<'a> CompressionSettings<'a> {
    pub fn independent_blocks(&mut self, v: bool) -> &mut Self {
        self.independent_blocks = v;
        self
    }
    pub fn block_checksums(&mut self, v: bool) -> &mut Self {
        self.block_checksums = v;
        self
    }
    pub fn content_checksum(&mut self, v: bool) -> &mut Self {
        self.content_checksum = v;
        self
    }
    /// Only valid values are 4MB, 1MB, 256KB, 64KB
    /// (TODO: better interface for this)
    pub fn block_size(&mut self, v: usize) -> &mut Self {
        self.block_size = v;
        self
    }
    pub fn dictionary(&mut self, id: u32, dict: &'a [u8]) -> &mut Self {
        self.dictionary = Some((id, dict));
        self
    }

    #[throws(io::Error)]
    pub fn compress<R: Read, W: Write>(&self, mut reader: R, mut writer: W) {
        use crate::compress::{U32Table, compress2, EncoderTable};

        let mut content_hasher = None;

        let mut flags = Flags::empty();
        if self.independent_blocks {
            flags |= Flags::IndependentBlocks;
        }
        if self.block_checksums {
            flags |= Flags::BlockChecksums;
        }
        if self.content_checksum {
            flags |= Flags::ContentChecksum;
            content_hasher = Some(XxHash32::with_seed(0));
        }
        if self.dictionary.is_some() { // TODO FIXME
            flags |= Flags::DictionaryId;
        }

        let version = 1 << 6;
        let flag_byte = version | flags.bits();
        let bd_byte = BlockDescriptor::new(self.block_size).0;

        let mut header = Vec::new();
        header.write_u32::<LE>(MAGIC)?;
        header.write_u8(flag_byte)?;
        header.write_u8(bd_byte)?;
        
        if flags.contains(Flags::ContentSize) {
            // FIXME depends on seek
        }

        let mut template_table = U32Table::default();
        let mut block_initializer: &[u8] = &[];
        if let Some((id, dict)) = self.dictionary {
//            header.write_u32::<LE>(id)?;

            compress2(dict, 0, &mut template_table, io::sink()).unwrap();
//            template_table.offset(dict.len());
            block_initializer = dict;
            println!("foo");
        }

        let mut hasher = XxHash32::with_seed(0);
        hasher.write(&header[4..]); // skip magic for header checksum
        header.write_u8((hasher.finish() >> 8) as u8)?;
        writer.write_all(&header)?;

        // TODO: when doing dependent blocks or dictionaries, in_buffer's capacity is insufficient
        let mut in_buffer = Vec::with_capacity(self.block_size);
        in_buffer.extend_from_slice(block_initializer);
        let mut out_buffer = vec![0u8; self.block_size];
        let mut table = template_table.clone();
        loop {
            let window_offset = in_buffer.len();

            // We basically want read_exact semantics, except at the end.
            // Sadly read_exact specifies the buffer contents to be undefined
            // on error, so we have to use this construction instead.
            reader.by_ref().take(in_buffer.capacity() as u64).read_to_end(&mut in_buffer)?;
            let read_bytes = in_buffer.len() - window_offset;
            if read_bytes == 0 {
                break;
            }

            if let Some(x) = content_hasher.as_mut() {
                x.write(&in_buffer[window_offset..]);
            }

            // TODO: implement u16 table for small inputs

            // 1. limit output by input size so we never have negative compression ratio
            // 2. use a wrapper that forbids partial writes, so don't write 32-bit integers
            //    as four individual bytes with four individual range checks
            let mut cursor = NoPartialWrites(&mut out_buffer[..read_bytes]);
            match compress2(&in_buffer, window_offset, &mut table, &mut cursor) {
                Ok(()) => {
                    let not_written_len = cursor.0.len();
                    let written_len = read_bytes - not_written_len;
//println!("{} -> {}", in_buffer.len(), written_len);
                    writer.write_u32::<LE>(written_len as u32)?;
                    writer.write_all(&out_buffer[..written_len])?;
                }
                Err(e) => {
                    assert!(e.kind() == ErrorKind::ConnectionAborted);
                    // incompressible
//println!("{} -> XXX", in_buffer.len());
                    writer.write_u32::<LE>((read_bytes as u32) | INCOMPRESSIBLE)?;
                    writer.write_all(&in_buffer[..read_bytes])?;
                }
            }

            if flags.contains(Flags::IndependentBlocks) {
                // clear table
                in_buffer.clear();
                in_buffer.extend_from_slice(block_initializer);

                table = template_table.clone();
            } else {
                if in_buffer.len() > WINDOW_SIZE {
                    let how_much_to_forget = in_buffer.len() - WINDOW_SIZE;
                    table.offset(how_much_to_forget);
                    in_buffer.drain(..how_much_to_forget);
                }
            }
        }
        writer.write_u32::<LE>(0)?;

        if let Some(x) = content_hasher {
            writer.write_u32::<LE>(x.finish() as u32)?;
        }
    }
    pub fn compress_with_size<R: Read + Seek>(&self, reader: R) {
//        reader.
    }
}

struct NoPartialWrites<'a>(&'a mut [u8]);
impl<'a> Write for NoPartialWrites<'a> {
    #[inline]
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.0.len() < data.len() {
            // quite frankly this doesn't matter
            return Err(ErrorKind::ConnectionAborted.into());
        }

        let amt = data.len();
        let (a, b) = mem::replace(&mut self.0, &mut []).split_at_mut(data.len());
        a.copy_from_slice(data);
        self.0 = b;
        Ok(amt)
    }

/*
    #[inline]
    fn write_all(&mut self, data: &[u8]) -> io::Result<()> {
        let amt = data.len();
        let (a, b) = mem::replace(&mut self.0, &mut []).split_at_mut(amt);
        a.copy_from_slice(&data[..amt]);
        self.0 = b;
        Ok(())
    }
*/

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}


const MAGIC: u32 = 0x184D2204;
const INCOMPRESSIBLE: u32 = 1 << 31;
const WINDOW_SIZE: usize = 64 * 1024;

bitflags! {
    struct Flags: u8 {
        const IndependentBlocks = 0b00100000;
        const BlockChecksums    = 0b00010000;
        const ContentSize       = 0b00001000;
        const ContentChecksum   = 0b00000100;
        const DictionaryId      = 0b00000001;
    }
}

pub type Flags2 = Flags;
pub type Bd2 = BlockDescriptor;

impl Flags {
    #[throws]
    fn parse(i: u8) -> Self {
        let version = i >> 6;
        if version != 1 {
            throw!(DecompressionError::UnsupportedVersion(version));
        }
        if (i & 0b10) != 0 {
            throw!(DecompressionError::ReservedFlagBitsSet);
        }

        Flags::from_bits_truncate(i)
    }

    fn independent_blocks(&self) -> bool { self.contains(Flags::IndependentBlocks) }
    fn block_checksums(&self)    -> bool { self.contains(Flags::BlockChecksums) }
    fn content_size(&self)       -> bool { self.contains(Flags::ContentSize) }
    fn content_checksum(&self)   -> bool { self.contains(Flags::ContentChecksum) }
    fn dictionary_id(&self)      -> bool { self.contains(Flags::DictionaryId) }
}

struct BlockDescriptor(pub u8); // ??? or what else could "BD" stand for ???
impl BlockDescriptor {
//    #[throws]
    fn new(block_maxsize: usize) -> Self {
        let mut v = 0;
        
        let maybe_maxsize = ((block_maxsize.trailing_zeros().saturating_sub(8)) / 2) as u8;
        let bd = BlockDescriptor::parse(maybe_maxsize << 4).unwrap();
        assert_eq!(block_maxsize, bd.block_maxsize().unwrap());

        bd
    }

    #[throws]
    fn parse(i: u8) -> Self {
        if (i & 0b10001111) != 0 {
            throw!(DecompressionError::ReservedBdBitsSet);
        }
        BlockDescriptor(i)
    }

    #[throws]
    fn block_maxsize(&self) -> usize {
        let size = (self.0 >> 4) & 0b111;
        if (4..8).contains(&size) {
            1 << (size * 2 + 8)
        } else {
            throw!(DecompressionError::UnimplementedBlocksize(size))
        }
    }
}

pub struct LZ4FrameIoReader<R: Read> {
    frame_reader: LZ4FrameReader<R>,
    bytes_taken: usize,
    buffer: Vec<u8>,
}
impl<R: Read> Read for LZ4FrameIoReader<R> {
    #[throws(IoError)]
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mybuf = self.fill_buf()?;
        let bytes_to_take = cmp::min(mybuf.len(), buf.len());
        &mut buf[..bytes_to_take].copy_from_slice(&mybuf[..bytes_to_take]);
        self.consume(bytes_to_take);
        bytes_to_take
    }
}
impl<R: Read> BufRead for LZ4FrameIoReader<R> {
    #[throws(IoError)]
    fn fill_buf(&mut self) -> &[u8] {
        if self.bytes_taken == self.buffer.len() {
            self.buffer.clear();
            self.frame_reader.decode_block(&mut self.buffer)?;
            self.bytes_taken = 0;
        }
        &self.buffer[self.bytes_taken..]
    }

    fn consume(&mut self, amt: usize) {
        self.bytes_taken += amt;
        assert!(self.bytes_taken <= self.buffer.len(), "You consumed more bytes than I even gave you!");
    }
}

pub struct LZ4FrameReader<R: Read> {
    reader: R,
    flags: Flags,
    block_maxsize: usize,
    read_buf: Vec<u8>,
    content_size: Option<u64>,
    dictionary_id: Option<u32>,
    content_hasher: Option<XxHash32>,
    carryover_window: Option<Vec<u8>>,
    finished: bool,
}

impl<R: Read> LZ4FrameReader<R> {
    #[throws]
    pub fn new(mut reader: R) -> Self {
        let magic = reader.read_u32::<LE>()?;
        if magic != MAGIC {
            throw!(DecompressionError::WrongMagic(magic));
        }

        let flags = Flags::parse(reader.read_u8()?)?;
        let bd = BlockDescriptor::parse(reader.read_u8()?)?;

        let mut hasher = XxHash32::with_seed(0);
        hasher.write_u8(flags.bits());
        hasher.write_u8(bd.0);

        let content_size = if flags.content_size() {
            let i = reader.read_u64::<LE>()?;
            hasher.write_u64(i);
            Some(i)
        } else {
            None
        };

        let dictionary_id = if flags.dictionary_id() {
            let i = reader.read_u32::<LE>()?;
            hasher.write_u32(i);
            Some(i)
        } else {
            None
        };

        let header_checksum_desired = reader.read_u8()?;
        let header_checksum_actual = (hasher.finish() >> 8) as u8;
        if header_checksum_desired != header_checksum_actual {
            throw!(DecompressionError::HeaderChecksumFail);
        }

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

        LZ4FrameReader {
            reader,
            flags,
            block_maxsize: bd.block_maxsize()?,
            content_size,
            dictionary_id,
            content_hasher,
            carryover_window,
            finished: false,
            read_buf: Vec::new()
        }
    }

    pub fn block_size(&self) -> usize { self.block_maxsize }
    pub fn frame_size(&self) -> Option<u64> { self.content_size }
    pub fn dictionary_id(&self) -> Option<u32> { self.dictionary_id }
    
    pub fn into_read(self) -> LZ4FrameIoReader<R> {
        LZ4FrameIoReader {
            buffer: Vec::with_capacity(self.block_size()),
            bytes_taken: 0,
            frame_reader: self,
        }
    }

    #[throws]
    pub fn decode_block(&mut self, output: &mut Vec<u8>) {
        assert!(output.is_empty(), "You must pass an empty buffer to this interface.");
        
        if self.finished { return; }

        let reader = &mut self.reader;

        let block_length = reader.read_u32::<LE>()?;
        if block_length == 0 {
            if let Some(hasher) = self.content_hasher.take() {
                let checksum = reader.read_u32::<LE>()?;
                if hasher.finish() != checksum.into() {
                    throw!(DecompressionError::FrameChecksumFail);
                }
            }
            self.finished = true;
            return;
        }

        let is_compressed = block_length & INCOMPRESSIBLE == 0;
        let block_length = block_length & !INCOMPRESSIBLE;

        let buf = &mut self.read_buf;
        buf.resize(block_length.try_into().or(Err(DecompressionError::BlockLengthOverflow))?, 0);
        reader.read_exact(buf.as_mut_slice())?;

        if self.flags.block_checksums() {
            let checksum = reader.read_u32::<LE>()?;
            let mut hasher = XxHash32::with_seed(0);
            hasher.write(&buf);
            if hasher.finish() != checksum.into() {
                throw!(DecompressionError::BlockChecksumFail);
            }
        }

        if is_compressed {
            if let Some(window) = self.carryover_window.as_mut() {
                decompress::decompress_block(&buf, &window, output)?;

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
            } else {
                decompress::decompress_block(&buf, &[], output)?;
            }
        } else {
            output.extend_from_slice(&buf);
        }

        if output.len() > self.block_maxsize {
            throw!(DecompressionError::BlockSizeOverflow);
        }

        if let Some(hasher) = self.content_hasher.as_mut() {
            hasher.write(&output);
        }
    }
}

#[throws]
pub fn decompress_file<R: Read>(reader: R) -> Vec<u8> {
    let mut reader = LZ4FrameReader::new(reader)?;

    let mut plaintext = Vec::new();

    let mut buf = Vec::with_capacity(reader.block_size());
    loop {
        reader.decode_block(&mut buf)?;
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
    use crate::compress::compress2;
    use crate::decompress::decompress;
    
    fn compress(input: &[u8]) -> Vec<u8> {
        let mut buf = Vec::new();
        if input.len() <= 0xFFFF {
            compress2::<_, crate::compress::U16Table>(input, &mut buf).unwrap();
        } else {
            compress2::<_, crate::compress::U32Table>(input, &mut buf).unwrap();
        }
        buf
    }

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
