use byteorder::{ReadBytesExt, LE};
use std::io::{self, Cursor, Read, ErrorKind};
use thiserror::Error;
use culpa::{throws, throw};

/// Errors when decoding a raw LZ4 block.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Error)]
pub enum DecodeError {
    #[error("Block stream ended prematurely. Either your input was truncated or you're trying to decompress garbage.")]
    UnexpectedEnd,
    #[error("Refusing to decode a repetition that would exceed the memory limit. If you're using framed mode, this is either garbage input or an OOM attack. If you're using raw mode, good luck figuring out whether this input is valid or not.")]
    MemoryLimitExceeded,
    #[error("The offset for a deduplication is zero. This is always invalid. You are probably decoding corrupted input.")]
    ZeroDeduplicationOffset,
    #[error("The offset for a deduplication is out of bounds. This may be caused by a missing or incomplete dictionary.")]
    InvalidDeduplicationOffset,
}
type Error = DecodeError; // do it this way for better docs

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Error {
        // this is the only kind of IO error that can happen in this code as we are always reading from slices
        assert_eq!(e.kind(), ErrorKind::UnexpectedEof);
        Error::UnexpectedEnd
    }
}

/// This is how LZ4 encodes varints.
/// Just keep reading and adding while it's all F
#[throws]
fn read_lsic(initial: u8, cursor: &mut Cursor<&[u8]>) -> usize {
    let mut value: usize = initial.into();
    if value == 0xF {
        loop {
            let more = cursor.read_u8()?;
            value += usize::from(more);
            if more != 0xff {
                break;
            }
        }
    }
    value
}

/// Decompress an LZ4-compressed block.
///
/// Note that LZ4 heavily relies on a lookback mechanism where bytes earlier in the output stream are referenced.
/// You may either pre-initialize the output buffer with this data or pass it separately in `prefix`.
/// In particular, an LZ4 "dictionary" should (probably) be implemented as a `prefix` because you obviously
/// don't want the dictionary to appear at the beginning of the output.
///
/// This function is based around memory buffers because that's what LZ4 intends.
/// If your blocks don't fit in your memory, you should use smaller blocks.
///
/// `output_limit` specifies a soft upper limit for the size of `output` (including
/// the data you passed on input). Note that this is only a measure to protect from
/// DoS attacks and in the worst case, we may exceed it by up to `input.len()` bytes.
#[throws]
pub fn decompress_raw(input: &[u8], prefix: &[u8], output: &mut Vec<u8>, output_limit: usize) {
    let mut reader = Cursor::new(input);
    while let Ok(token) = reader.read_u8() {
        // read literals
        let literal_length = read_lsic(token >> 4, &mut reader)?;

        let output_pos_pre_literal = output.len();
        output.resize(output_pos_pre_literal + literal_length, 0);
        reader.read_exact(&mut output[output_pos_pre_literal..])?;

        // read duplicates
        if let Ok(offset) = reader.read_u16::<LE>() {
            let match_len = 4 + read_lsic(token & 0xf, &mut reader)?;
            if (output.len() + match_len) > output_limit {
                throw!(Error::MemoryLimitExceeded);
            }
            copy_overlapping(offset.into(), match_len, prefix, output)?;
        }
    }
}

fn copy_overlapping(offset: usize, match_len: usize, prefix: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
    let old_len = output.len();
    match offset {
        0 => return Err(Error::ZeroDeduplicationOffset),
        i if i > old_len => {
            // need prefix for this
            let prefix_needed = i - old_len;
            if prefix_needed > prefix.len() {
                return Err(Error::InvalidDeduplicationOffset);
            }
            let how_many_bytes_from_prefix = std::cmp::min(prefix_needed, match_len);
            output.extend_from_slice(
                &prefix[prefix.len() - prefix_needed..][..how_many_bytes_from_prefix],
            );
            let remaining_len = match_len - how_many_bytes_from_prefix;
            if remaining_len != 0 {
                // offset stays the same because our curser moved forward by the amount of bytes we took from prefix
                return copy_overlapping(offset, remaining_len, &[], output);
            }
        }

        // fastpath: memset if we repeat the same byte forever
        1 => output.resize(old_len + match_len, output[old_len - 1]),

        o if match_len <= o => {
            // fastpath: nonoverlapping
            // for borrowck reasons we have to extend with zeroes first and then memcpy
            // instead of simply using extend_from_slice
            output.resize(old_len + match_len, 0);
            let (head, tail) = output.split_at_mut(old_len);
            tail.copy_from_slice(&head[old_len - offset..][..match_len]);
        }
        2 | 4 | 8 => {
            // fastpath: overlapping but small

            // speedup: build 16 byte buffer so we can handle 16 bytes each iteration instead of one
            let mut buf = [0u8; 16];
            for chunk in buf.chunks_mut(offset) {
                // if this panics (i.e. chunklen != delta), delta does not divide 16 (but it always does)
                chunk.copy_from_slice(&output[old_len - offset..][..offset]);
            }
            // fill with zero bytes
            output.resize(old_len + match_len, 0);
            // copy buf as often as possible
            for target in output[old_len..].chunks_mut(buf.len()) {
                target.copy_from_slice(&buf[..target.len()]);
            }
        }
        _ => {
            // slowest path: copy single bytes
            output.reserve(match_len);
            for i in 0..match_len {
                let b = output[old_len - offset + i];
                output.push(b);
            }
        }
    }
    Ok(())
}


#[cfg(test)]
pub mod test {
    use fehler::throws;
    use super::{decompress_raw, Error};

    #[throws]
    pub fn decompress(input: &[u8]) -> Vec<u8> {
        let mut vec = Vec::new();
        decompress_raw(input, &[], &mut vec, std::usize::MAX)?;
        vec
    }

    #[test]
    fn aaaaaaaaaaa_lots_of_aaaaaaaaa() {
        assert_eq!(decompress(&[0x11, b'a', 1, 0]).unwrap(), b"aaaaaa");
    }

    #[test]
    fn multiple_repeated_blocks() {
        assert_eq!(
            decompress(&[0x11, b'a', 1, 0, 0x22, b'b', b'c', 2, 0]).unwrap(),
            b"aaaaaabcbcbcbc"
        );
    }

    #[test]
    fn all_literal() {
        assert_eq!(decompress(&[0x30, b'a', b'4', b'9']).unwrap(), b"a49");
    }

    #[test]
    fn offset_oob() {
        decompress(&[0x10, b'a', 2, 0]).unwrap_err();
        decompress(&[0x40, b'a', 1, 0]).unwrap_err();
    }
}
