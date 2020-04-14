//! The compression algorithm.
//!
//! We make use of hash tables to find duplicates. This gives a reasonable compression ratio with a
//! high performance. It has fixed memory usage, which contrary to other approachs, makes it less
//! memory hungry.

use std::mem;
use std::cmp;
use std::io::Write;
use std::convert::TryInto;
use byteorder::{ByteOrder, NativeEndian, WriteBytesExt, LE};
use fehler::{throws};

type Error = std::io::Error;

/// Duplication dictionary size.
///
/// Every four bytes is assigned an entry. When this number is lower, fewer entries exists, and
/// thus collisions are more likely, hurting the compression ratio.
const DICTIONARY_SIZE: usize = 1 << HASHLOG;
const HASHLOG: usize = 12;
const MINMATCH: usize = 4;


pub trait EncoderTable: Default {
    fn get(&self, key: &[u8]) -> usize;
    // value is declared as usize but must not be above payload_size_limit
    fn set(&mut self, key: &[u8], value: usize);
    fn payload_size_limit() -> usize;
}

pub struct U32Table {
    dict: [u32; DICTIONARY_SIZE],
}
impl Default for U32Table {
    fn default() -> Self {
        U32Table { dict: [0; DICTIONARY_SIZE] }
    }
}


fn hash5(input: &[u8]) -> usize {
    // read 64 bits as 4+1 bytes
    let upper_byte = input.get(4).copied().unwrap_or(0);
    let v = NativeEndian::read_u32(input) as u64 | ((upper_byte as u64) << 32);

    // calculate a checksum
    ((v << 24).wrapping_mul(889523592379) as usize) >> (64 - HASHLOG)
}

impl EncoderTable for U32Table {
    fn get(&self, key: &[u8]) -> usize {
        self.dict[hash5(key)].try_into().expect("This code is not supposed to run on a 16-bit arch (let alone smaller)")
    }
    fn set(&mut self, key: &[u8], value: usize) {
        self.dict[hash5(key)] = value.try_into().expect("EncoderTable contract violated");
    }
    fn payload_size_limit() -> usize { u32::MAX as usize }
}
/*
struct U16Table {
    dict: [u16; DICTIONARY_SIZE*2],
}
impl EncoderTable for U16Table {
     fn payload_size_limit() -> usize { u16::MAX as usize }
}
*/

/// A consecutive sequence of bytes found in already encoded part of the input.
#[derive(Copy, Clone, Debug)]
struct Duplicate {
    /// The number of bytes before our cursor, where the duplicate starts.
    offset: u16,

    /// The length beyond the four first bytes.
    ///
    /// Adding four to this number yields the actual length.
    extra_bytes: usize,
}




fn count_matching_bytes(a: &[u8], b: &[u8]) -> usize {
    const REGSIZE: usize = mem::size_of::<usize>();
    fn read_usize(b: &[u8]) -> usize { // sadly byteorder doesn't have this
        let mut buf = [0u8; REGSIZE];
        buf.copy_from_slice(&b[..REGSIZE]);
        usize::from_le_bytes(buf)
    }
    #[cfg(target_endian = "little")] fn archdep_zeros(i: usize) -> u32 { i.trailing_zeros() }
    #[cfg(target_endian = "big")] fn archdep_zeros(i: usize) -> u32 { i.leading_zeros() }

    let mut matching_bytes = 0;
    // match in chunks of usize so we process a full register at a time instead of single bytes
    for (a, b) in a.chunks_exact(REGSIZE).zip(b.chunks_exact(REGSIZE)) {
        let a = read_usize(a);
        let b = read_usize(b);
        let xor = a ^ b;
        if xor == 0 {
            matching_bytes += REGSIZE;
        } else {
            matching_bytes += (archdep_zeros(xor) / 8/*bits per byte*/) as usize;
            return matching_bytes;
        }
    }
    
    // we only return here if we ran out of data (i.e. all 4-byte blocks have matched)
    // but there may be up to 3 more bytes to check!
    let trailing_matches = a.iter().zip(b).skip(matching_bytes).take_while(|&(a, b)| a == b).count();
    matching_bytes + trailing_matches
}

const ACCELERATION: usize = 1;
const SKIP_TRIGGER: usize = 6; // for each 64 steps, skip in bigger increments

#[throws]
pub fn compress2<W: Write, T: EncoderTable>(input: &[u8], mut writer: W) {
    let mut table = T::default();

    let mut cursor = 0;
    while cursor < input.len() {
        let literal_start = cursor;

        let mut step_counter = ACCELERATION << SKIP_TRIGGER;
        let mut step = 1;
        // look for a duplicate
        let duplicate = loop {
            if (input.len() - cursor) < 13 {
                // end with a literal-only section
                // the limit of 13 bytes is somewhat arbitrarily chosen by the spec (our decoder doesn't need it)
                // probably to allow some insane decoder optimization they do in C
                let literal_len = input.len() - literal_start;
                
                let mut token = 0;
                write_lsic_head(&mut token, 4, input.len() - literal_start);
                writer.write_u8(token)?;
                write_lsic_tail(&mut writer, literal_len)?;
                writer.write_all(&input[literal_start..][..literal_len])?;
                return;
            }

            // due to the check above we know there's at least 13 bytes of space
            // we have to chop off the last five bytes though because the spec also (completely arbitrarily, I must say)
            // requires these to be encoded as literals (once again, our decoder does not require this)
            let current_batch = &input[cursor..(input.len() - 5)];
            let candidate = table.get(current_batch);
            table.set(current_batch, cursor);

            if (cursor != 0) // can never match on the very first byte
                && cursor - candidate <= 0xFFFF { // must be an addressable offset
                // let's see how many matching bytes we have
                let candidate_batch = &input[candidate..];
                let matching_bytes = count_matching_bytes(current_batch, candidate_batch);

                if let Some(mut extra_bytes) = matching_bytes.checked_sub(MINMATCH) {
                    // if it wasn't, this was just a hash collision :(
                    let offset = (cursor - candidate) as u16;

                    // backtrack
                    let max_backtrack = cmp::min(cursor - literal_start, (u16::MAX - offset) as usize);
                    let backtrack = input[..cursor].iter().rev().zip(input[..candidate].iter().rev()).take(max_backtrack).take_while(|&(a, b)| a == b).count();
                    // offset remains unchanged
                    extra_bytes += backtrack;
                    cursor += matching_bytes;

                    // not sure why exactly this, but that's what they do
                    let minus_two = &input[cursor-2..];
                    if minus_two.len() >= 4 {
                        table.set(minus_two, cursor-2);
                    }
        
                    break Duplicate { offset, extra_bytes };
                }
            }
            
            // no match, keep looping
            cursor += step;
            step = step_counter >> SKIP_TRIGGER;

            // the first byte of each iteration doesn't count due to some weird-ass manual loop unrolling in the C code
            if literal_start+1 != cursor {
                step_counter += 1
            }
        };
        
        // cursor is now pointing past the match
        let literal_end = cursor - duplicate.extra_bytes - MINMATCH;
        let literal_len = literal_end - literal_start;
        
        let mut token = 0;
        write_lsic_head(&mut token, 4, literal_len);
        write_lsic_head(&mut token, 0, duplicate.extra_bytes);

        writer.write_u8(token)?;
        write_lsic_tail(&mut writer, literal_len)?;
        writer.write_all(&input[literal_start..literal_end])?;
        writer.write_u16::<LE>(duplicate.offset)?;
        write_lsic_tail(&mut writer, duplicate.extra_bytes)?;
   }
}
fn write_lsic_head(token: &mut u8, shift: usize, value: usize) {
    let i = cmp::min(value, 0xF) as u8;
    *token |= i << shift;
}
#[throws]
fn write_lsic_tail<W: Write>(writer: &mut W, mut value: usize) {
    if value < 0xF {
        return;
    }

    value -= 0xF;

    while value >= 4*0xFF {
        writer.write_u32::<NativeEndian>(u32::MAX)?;
        value -= 4*0xFF;
    }
    while value >= 0xFF {
        writer.write_u8(0xFF)?;
        value -= 0xFF;
    }
    writer.write_u8(value as u8)?;
}

