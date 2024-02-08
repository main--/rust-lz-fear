use byteorder::{LE, ReadBytesExt};
use std::hash::Hasher;
use std::io::{self, Read, BufRead, ErrorKind};
use std::cmp;
use std::convert::TryInto;
use twox_hash::XxHash32;
use thiserror::Error;
use culpa::{throw, throws};

use super::{MAGIC, INCOMPRESSIBLE, WINDOW_SIZE};
use super::header::{self, Flags, BlockDescriptor};
use crate::raw;


/// Errors when decompressing an LZ4 frame.
#[derive(Error, Debug)]
pub enum DecompressionError {
    #[error("error reading from the input you gave me")]
    InputError(#[from] io::Error),
    #[error("the raw LZ4 decompression failed (data corruption?)")]
    CodecError(#[from] raw::DecodeError),
    #[error("invalid header")]
    HeaderParseError(#[from] header::ParseError),
    #[error("wrong magic number in file header: {0:08x}")]
    WrongMagic(u32),
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
type Error = DecompressionError; // do it this way for better docs

impl From<Error> for io::Error {
    fn from(e: Error) -> io::Error {
        io::Error::new(ErrorKind::Other, e)
    }
}

/// Wrapper around `LZ4FrameReader` that implements `Read` and `BufRead`.
pub struct LZ4FrameIoReader<'a, R: Read> {
    frame_reader: LZ4FrameReader<R>,
    bytes_taken: usize,
    buffer: Vec<u8>,
    dictionary: &'a [u8],
}
impl<R: Read> Read for LZ4FrameIoReader<'_, R> {
    #[throws(io::Error)]
    fn read(&mut self, buf: &mut [u8]) -> usize {
        let mybuf = self.fill_buf()?;
        let bytes_to_take = cmp::min(mybuf.len(), buf.len());
        buf[..bytes_to_take].copy_from_slice(&mybuf[..bytes_to_take]);
        self.consume(bytes_to_take);
        bytes_to_take
    }
}
impl<R: Read> BufRead for LZ4FrameIoReader<'_, R> {
    #[throws(io::Error)]
    fn fill_buf(&mut self) -> &[u8] {
        if self.bytes_taken == self.buffer.len() {
            self.buffer.clear();
            self.frame_reader.decode_block(&mut self.buffer, self.dictionary)?;
            self.bytes_taken = 0;
        }
        &self.buffer[self.bytes_taken..]
    }

    fn consume(&mut self, amt: usize) {
        self.bytes_taken += amt;
        assert!(self.bytes_taken <= self.buffer.len(), "You consumed more bytes than I even gave you!");
    }
}

/// Read an LZ4-compressed frame.
///
/// This reader reads the blocks inside a frame one by one.
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
    /// Create a new LZ4FrameReader over an underlying reader and parse the header.
    ///
    /// A typical LZ4 file consists of exactly one frame.
    /// This reader will stop reading at the end of the frame.
    /// If you want to read any data following this frame, you should probably
    /// pass in your reader by reference, rather than by value.
    #[throws]
    pub fn new(mut reader: R) -> Self {
        let magic = reader.read_u32::<LE>()?;
        if magic != MAGIC {
            throw!(Error::WrongMagic(magic));
        }

        let flags_byte = reader.read_u8()?;
        let flags = Flags::parse(flags_byte)?;
        let bd = BlockDescriptor::parse(reader.read_u8()?)?;

        let mut hasher = XxHash32::with_seed(0);
        hasher.write_u8(flags_byte);
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
            throw!(Error::HeaderChecksumFail);
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

    /// Returns the maximum number of bytes a block can decompress to (as specified by the file header).
    ///
    /// In general, all blocks in a frame except for the final one will have exactly this size.
    /// (Although this is not strictly enforced and may be violated by hand-crafted inputs)
    pub fn block_size(&self) -> usize { self.block_maxsize }
    /// Returns the number of bytes that this entire frame is supposed to decompress to.
    /// This value is read directly from the file header and may be incorrect for malicious inputs.
    pub fn frame_size(&self) -> Option<u64> { self.content_size }
    /// Return an identifier for the dictionary that was used to compress this frame.
    ///
    /// Dictionary identifiers are always application-specific. Note that the lz4 command line utility never
    /// specifies a dictionary id, even if a dictionary was used.
    pub fn dictionary_id(&self) -> Option<u32> { self.dictionary_id }

    /// Convert this `LZ4FrameReader` into something that implements `std::io::BufRead`.
    ///
    /// Note that `io::copy` has a small performance issue: https://github.com/rust-lang/rust/issues/49921
    pub fn into_read_with_dictionary(self, dictionary: &[u8]) -> LZ4FrameIoReader<R> {
        LZ4FrameIoReader {
            buffer: Vec::with_capacity(self.block_size()),
            bytes_taken: 0,
            frame_reader: self,
            dictionary,
        }
    }

    /// Convenience wrapper in case you don't want to specify a dictionary.
    pub fn into_read(self) -> LZ4FrameIoReader<'static, R> {
        self.into_read_with_dictionary(&[])
    }

    /// Decode a single block.
    ///
    /// The `output` buffer must be empty upon calling this method.
    #[throws]
    pub fn decode_block(&mut self, output: &mut Vec<u8>, dictionary: &[u8]) {
        assert!(output.is_empty(), "You must pass an empty buffer to this interface.");
        
        if self.finished { return; }

        let reader = &mut self.reader;

        let block_length = reader.read_u32::<LE>()?;
        if block_length == 0 {
            if let Some(hasher) = self.content_hasher.take() {
                let checksum = reader.read_u32::<LE>()?;
                if hasher.finish() != checksum.into() {
                    throw!(Error::FrameChecksumFail);
                }
            }
            self.finished = true;
            return;
        }

        let is_compressed = block_length & INCOMPRESSIBLE == 0;
        let block_length = block_length & !INCOMPRESSIBLE;

        if block_length > self.block_maxsize as u32 {
            throw!(Error::BlockSizeOverflow);
        }

        let buf = &mut self.read_buf;
        buf.resize(block_length.try_into().or(Err(Error::BlockLengthOverflow))?, 0);
        reader.read_exact(buf.as_mut_slice())?;

        if self.flags.block_checksums() {
            let checksum = reader.read_u32::<LE>()?;
            let mut hasher = XxHash32::with_seed(0);
            hasher.write(buf);
            if hasher.finish() != checksum.into() {
                throw!(Error::BlockChecksumFail);
            }
        }

        // set up the prefix properly
        let dec_prefix = if let Some(window) = self.carryover_window.as_mut() {
            if window.is_empty() {
                window.extend_from_slice(dictionary);
            }
            window
        } else {
            dictionary
        };
        // decompress or copy, depending on whether this block is compressed
        if is_compressed {
            raw::decompress_raw(buf, dec_prefix, output, self.block_maxsize)?;
        } else {
            output.extend_from_slice(buf);
        }
        // finally, push data back into the window as needed
        if let Some(window) = self.carryover_window.as_mut() {
            let outlen = output.len();
            if outlen < WINDOW_SIZE {
                let available_bytes = window.len() + outlen;
                if let Some(surplus_bytes) = available_bytes.checked_sub(WINDOW_SIZE) {
                    // remove as many bytes from front as we are replacing
                    window.drain(..surplus_bytes);
                }
                window.extend_from_slice(output);
            } else {
                // TODO: optimize this case to avoid the copy
                window.clear();
                window.extend_from_slice(&output[outlen - WINDOW_SIZE..]);
            }

            assert!(window.len() <= WINDOW_SIZE);
        }


        if output.len() > self.block_maxsize {
            throw!(Error::BlockSizeOverflow);
        }

        if let Some(hasher) = self.content_hasher.as_mut() {
            hasher.write(output);
        }
    }
}

/// Convenience wrapper around `LZ4FrameReader` that reads everything into a vector and returns it.
#[throws]
pub fn decompress_frame<R: Read>(reader: R) -> Vec<u8> {
    let mut plaintext = Vec::new();
    LZ4FrameReader::new(reader)?.into_read().read_to_end(&mut plaintext)?;
    plaintext
}

