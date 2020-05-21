use byteorder::{LE, WriteBytesExt};
use std::hash::Hasher;
use std::io::{self, Read, Write, Seek, SeekFrom, ErrorKind};
use std::mem;
use twox_hash::XxHash32;
use thiserror::Error;
use fehler::{throws};

use super::{MAGIC, INCOMPRESSIBLE, WINDOW_SIZE};
use super::header::{Flags, BlockDescriptor};
use crate::raw::{U32Table, compress2, EncoderTable};


/// Errors when compressing an LZ4 frame.
#[derive(Error, Debug)]
pub enum CompressionError {
    #[error("error reading from the input you gave me")]
    ReadError(io::Error),
    #[error("error writing to the output you gave me")]
    WriteError(#[from] io::Error),
    #[error("the block size you asked for is not supported")]
    InvalidBlockSize,
}
type Error = CompressionError; // do it this way for better docs
impl From<Error> for io::Error {
    fn from(e: Error) -> io::Error {
        io::Error::new(ErrorKind::Other, e)
    }
}

/// A builder-style struct that configures compression settings.
/// This is how you compress LZ4 frames.
/// (An LZ4 file usually consists of a single frame.)
///
/// Create it using `Default::default()`.
pub struct CompressionSettings<'a> {
    independent_blocks: bool,
    block_checksums: bool,
    content_checksum: bool,
    block_size: usize,
    dictionary: Option<&'a [u8]>,
    dictionary_id: Option<u32>,
}
impl<'a> Default for CompressionSettings<'a> {
    fn default() -> Self {
        Self {
            independent_blocks: true,
            block_checksums: false,
            content_checksum: true,
            block_size: 4 * 1024 * 1024,
            dictionary: None,
            dictionary_id: None,
        }
    }
}
impl<'a> CompressionSettings<'a> {
    /// In independent mode, blocks are not allowed to reference data from previous blocks.
    /// Hence, using dependent blocks yields slightly better compression.
    /// The downside of dependent blocks is that seeking becomes impossible - the entire frame always has
    /// to be decompressed from the beginning.
    ///
    /// Blocks are independent by default.
    pub fn independent_blocks(&mut self, v: bool) -> &mut Self {
        self.independent_blocks = v;
        self
    }

    /// Block checksums can help detect data corruption in storage and transit.
    /// They do not offer error correction though.
    ///
    /// In most cases, block checksums are not very helpful because you generally want a lower
    /// layer to deal with data corruption more comprehensively.
    ///
    /// Block checksums are disabled by default.
    pub fn block_checksums(&mut self, v: bool) -> &mut Self {
        self.block_checksums = v;
        self
    }

    /// The content checksum (also called frame checksum) is calculated over the contents of the entire frame.
    /// This makes them cheaper than block checksums as their size overhead is constant
    /// as well as marginally more useful, because they can help protect against incorrect decompression.
    ///
    /// Note that the content checksum can only be verified *after* the entire frame has been read
    /// (and returned!), which is the downside of content checksums.
    ///
    /// Frame checksums are enabled by default.
    pub fn content_checksum(&mut self, v: bool) -> &mut Self {
        self.content_checksum = v;
        self
    }

    /// Only valid values are 4MiB, 1MiB, 256KiB, 64KiB
    /// (TODO: better interface for this)
    ///
    /// The default block size is 4 MiB.
    pub fn block_size(&mut self, v: usize) -> &mut Self {
        self.block_size = v;
        self
    }

    /// A dictionary is essentially a constant slice of bytes shared by the compressing and decompressing party.
    /// Using a dictionary can improve compression ratios, because the compressor can reference data from the dictionary.
    ///
    /// The dictionary id is an application-specific identifier which can be used during decompression to determine
    /// which dictionary to use.
    ///
    /// Note that while the size of a dictionary can be arbitrary, dictionaries larger than 64 KiB are not useful as
    /// the LZ4 algorithm does not support backreferences by more than 64 KiB, i.e. any dictionary content before
    /// the trailing 64 KiB is silently ignored.
    ///
    /// By default, no dictionary is used and no id is specified.
    pub fn dictionary(&mut self, id: u32, dict: &'a [u8]) -> &mut Self {
        self.dictionary_id = Some(id);
        self.dictionary = Some(dict);
        self
    }

    /// The dictionary id header field is quite obviously intended to tell anyone trying to decompress your frame which dictionary to use.
    /// So it is only natural to assume that the *absence* of a dictionary id indicates that no dictionary was used.
    ///
    /// Unfortunately this assumption turns out to be incorrect. The LZ4 CLI simply never writes a dictionary id.
    /// The major downside is that you can no longer distinguish corrupted data from a missing dictionary
    /// (unless you write block checksums, which the LZ4 CLI also never does).
    ///
    /// Hence, this library is opinionated in the sense that we always want you to specify either neither or both of these things
    /// (the LZ4 CLI basically just ignores the dictionary id completely and only cares about whether you specify a dictionary parameter or not).
    ///
    /// If you think you know better (you probably don't) you may use this method to break this rule.
    pub fn dictionary_id_nonsense_override(&mut self, id: Option<u32>) -> &mut Self {
        self.dictionary_id = id;
        self
    }

    // TODO: these interfaces need to go away in favor of something that can handle individual blocks rather than always compressing full frames at once

    #[throws]
    pub fn compress<R: Read, W: Write>(&self, reader: R, writer: W) {
        self.compress_internal(reader, writer, None)?;
    }

    #[throws]
    pub fn compress_with_size_unchecked<R: Read, W: Write>(&self, reader: R, writer: W, content_size: u64) {
        self.compress_internal(reader, writer, Some(content_size))?;
    }

    #[throws]
    pub fn compress_with_size<R: Read + Seek, W: Write>(&self, mut reader: R, writer: W) {
        // maybe one day we can just use reader.stream_len() here: https://github.com/rust-lang/rust/issues/59359
        // then again, we implement this to ignore the all bytes before the cursor which stream_len() does not
        let start = reader.seek(SeekFrom::Current(0))?;
        let end = reader.seek(SeekFrom::End(0))?;
        reader.seek(SeekFrom::Start(start))?;

        let length = end - start;
        self.compress_internal(reader, writer, Some(length))?;
    }

    #[throws]
    fn compress_internal<R: Read, W: Write>(&self, mut reader: R, mut writer: W, content_size: Option<u64>) {
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
        if self.dictionary_id.is_some() {
            flags |= Flags::DictionaryId;
        }
        if content_size.is_some() {
            flags |= Flags::ContentSize;
        }

        let version = 1 << 6;
        let flag_byte = version | flags.bits();
        let bd_byte = BlockDescriptor::new(self.block_size).ok_or(Error::InvalidBlockSize)?.0;

        let mut header = Vec::new();
        header.write_u32::<LE>(MAGIC)?;
        header.write_u8(flag_byte)?;
        header.write_u8(bd_byte)?;
        
        if let Some(content_size) = content_size {
            header.write_u64::<LE>(content_size)?;
        }
        if let Some(id) = self.dictionary_id {
            header.write_u32::<LE>(id)?;
        }

        let mut hasher = XxHash32::with_seed(0);
        hasher.write(&header[4..]); // skip magic for header checksum
        header.write_u8((hasher.finish() >> 8) as u8)?;
        writer.write_all(&header)?;

        let mut template_table = U32Table::default();
        let mut block_initializer: &[u8] = &[];
        if let Some(dict) = self.dictionary {
            for window in dict.windows(mem::size_of::<usize>()).step_by(3) {
                // this is a perfectly safe way to find out where our window is pointing
                // we could do this manually by iterating with an index to avoid the scary-looking
                // pointer math but this is way more convenient IMO
                let offset = window.as_ptr() as usize - dict.as_ptr() as usize;
                template_table.replace(dict, offset);
            }

            block_initializer = dict;
        }

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
            reader.by_ref().take(self.block_size as u64).read_to_end(&mut in_buffer).map_err(Error::ReadError)?;
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
            let write = match compress2(&in_buffer, window_offset, &mut table, &mut cursor) {
                Ok(()) => {
                    let not_written_len = cursor.0.len();
                    let written_len = read_bytes - not_written_len;
                    writer.write_u32::<LE>(written_len as u32)?;
                    &out_buffer[..written_len]
                }
                Err(e) => {
                    assert!(e.kind() == ErrorKind::ConnectionAborted);
                    // incompressible
                    writer.write_u32::<LE>((read_bytes as u32) | INCOMPRESSIBLE)?;
                    &in_buffer[window_offset..]
                }
            };

            writer.write_all(write)?;
            if flags.contains(Flags::BlockChecksums) {
                let mut block_hasher = XxHash32::with_seed(0);
                block_hasher.write(write);
                writer.write_u32::<LE>(block_hasher.finish() as u32)?;
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
}

/// Helper struct to allow more efficient code generation when using the Write trait on byte buffers.
///
/// The underlying problem is that the Write impl on [u8] (and everything similar, e.g. Cursor<[u8]>)
/// is specified to write as many bytes as possible before returning an error.
/// This is a problem because it forces e.g. a 32-bit write to compile to four 8-bit writes with a range
/// check every time, rather than a single 32-bit write with a range check.
///
/// This wrapper aims to resolve the problem by simply not writing anything in case we fail the bounds check,
/// as we throw away the entire buffer in that case anyway.
struct NoPartialWrites<'a>(&'a mut [u8]);
impl<'a> Write for NoPartialWrites<'a> {
    #[inline]
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        if self.0.len() < data.len() {
            // quite frankly it doesn't matter what we specify here
            return Err(ErrorKind::ConnectionAborted.into());
        }

        let amt = data.len();
        let (a, b) = mem::replace(&mut self.0, &mut []).split_at_mut(data.len());
        a.copy_from_slice(data);
        self.0 = b;
        Ok(amt)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

