use byteorder::{ReadBytesExt, LE};
use std::io::{Cursor, Read};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub enum Error {
    /// Expected more bytes, but found none.
    /// Either your input was truncated or you're trying to decompress garbage.
    UnexpectedEnd,
    /// The offset for a deduplication is out of bounds.
    /// This may be caused by a missing or incomplete dictionary.
    InvalidDeduplicationOffset,
}

/// This is how LZ4 encodes varints.
/// Just keep reading and adding while it's all F
fn read_lsic(initial: u8, cursor: &mut Cursor<&[u8]>) -> u64 {
    let mut value = initial as u64;
    if value == 0xF {
        loop {
            let more = cursor.read_u8().unwrap();
            value += more as u64;
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
pub fn decompress_block(input: &[u8], prefix: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
    let mut reader = Cursor::new(input);
    loop {
        let token = match reader.read_u8() {
            Ok(x) => x,
            _ => break,
        };

        // read literals
        let literal_length = read_lsic(token >> 4, &mut reader) as usize;

        let output_pos_pre_literal = output.len();
        output.resize(output_pos_pre_literal + literal_length, 0);
        if let Err(_) = reader.read_exact(&mut output[output_pos_pre_literal..]) {
            return Err(Error::UnexpectedEnd);
        }

        // read duplicates
        let offset = match reader.read_u16::<LE>() {
            Ok(x) => x,
            _ => break,
        } as usize;
        let match_len = 4 + read_lsic(token & 0xf, &mut reader) as usize;
        copy_overlapping(offset, match_len, prefix, output)?;
    }
    Ok(())
}

fn copy_overlapping(
    offset: usize,
    match_len: usize,
    prefix: &[u8],
    output: &mut Vec<u8>,
) -> Result<(), Error> {
    let old_len = output.len();
    match offset {
        0 => unreachable!("invalid offset"),
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

/// Decompress all bytes of `input`.
pub fn decompress(input: &[u8]) -> Result<Vec<u8>, Error> {
    // Allocate a vector to contain the decompressed stream.
    let mut vec = Vec::new();
    decompress_block(input, &[], &mut vec)?;
    Ok(vec)
}

#[cfg(test)]
mod test {
    use super::*;

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
