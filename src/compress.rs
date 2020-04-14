//! The compression algorithm.
//!
//! We make use of hash tables to find duplicates. This gives a reasonable compression ratio with a
//! high performance. It has fixed memory usage, which contrary to other approachs, makes it less
//! memory hungry.

use std::cmp;
use std::io::Read;
use std::convert::TryInto;
use byteorder::{ByteOrder, NativeEndian, ReadBytesExt, WriteBytesExt, LE};
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



#[cfg(target_endian = "little")] fn archdep_zeros(i: u64) -> u32 { i.trailing_zeros() }
#[cfg(target_endian = "big")] fn archdep_zeros(i: u64) -> u32 { i.leading_zeros() }

fn count_matching_bytes(a: &[u8], b: &[u8]) -> usize {
    let mut matching_bytes = 0;
    // match in chunks of 4 bytes so we process an 32 bits at a time
    for (a, b) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        let a = NativeEndian::read_u64(a);
        let b = NativeEndian::read_u64(b);
        let xor = a ^ b;
        if xor == 0 {
            matching_bytes += 8;
        } else {
//            if matching_bytes == 0 && (xor as u32 != 0) { return 0; }
            matching_bytes += (archdep_zeros(xor) / 8) as usize;
            return matching_bytes;
        }
    }
    
    // we only return here if we ran out of data (i.e. all 4-byte blocks have matched)
    // but there may be up to 3 more bytes to check!
    let trailing_matches = a.iter().zip(b).skip(matching_bytes).take_while(|&(a, b)| a == b).count();
    //let trailing_matches = a[matching_bytes..].iter().zip(&b[matching_bytes..]).take_while(|&(a, b)| a == b).count();
    matching_bytes + trailing_matches
}
/*
#[cfg(target_endian = "little")] fn archdep_zeros(i: u32) -> u32 { i.trailing_zeros() }
#[cfg(target_endian = "big")] fn archdep_zeros(i: u32) -> u32 { i.leading_zeros() }

fn count_matching_bytes(a: &[u8], b: &[u8]) -> usize {
     let mut matching_bytes = 0;
     // match in chunks of 4 bytes so we process an 32 bits at a time
     for (a, b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        let a = NativeEndian::read_u32(a);
        let b = NativeEndian::read_u32(b);
        let xor = a ^ b;
        if xor == 0 {
            matching_bytes += 4;
        } else {
            // optimization: if it doesn't match at all, don't even bother
            // TODO: benchmark whether this is worth
            if matching_bytes != 0 {
                matching_bytes += archdep_zeros(xor) as usize / 8;
            }
            return matching_bytes;
        }
    }
    
    // we only return here if we ran out of data (i.e. all 4-byte blocks have matched)
    // but there may be up to 3 more bytes to check!
    let trailing_matches = a.iter().zip(b).skip(matching_bytes).take_while(|&(a, b)| a == b).count();
    //let trailing_matches = a[matching_bytes..].iter().zip(&b[matching_bytes..]).take_while(|&(a, b)| a == b).count();
    matching_bytes + trailing_matches
}
*/

use std::io::{Write, Cursor};
#[throws]
pub fn compress2<W: Write, T: EncoderTable>(input: &[u8], mut writer: W) {
    let mut table = T::default();

    let mut cursor = 0;
    while cursor < input.len() {
        let literal_start = cursor;
        
        let LZ4_skipTrigger = 6;
        let mut searchMatchNb = /*acceleration*/1 << LZ4_skipTrigger;
        let mut step = 1;
        // look for a duplicate
        let duplicate = loop {
            if (input.len() - cursor) < 4 {
                // end with a literal-only section
                let literal_len = input.len() - literal_start;
                
                let mut token = 0;
                write_lsic_head(&mut token, 4, input.len() - literal_start);
                writer.write_u8(token)?;
                write_lsic_tail(&mut writer, literal_len)?;
                writer.write_all(&input[literal_start..][..literal_len])?;
                return;
            }
        
            let current_batch = &input[cursor..];
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
                    
//                    if backtrack > 0 {
//                    table.set(&input[cursor-backtrack-1..], cursor-backtrack-1);
//                    }

                    cursor += matching_bytes;

/*
        let literal_end = cursor - extra_bytes - MINMATCH;
        let literal_len = literal_end - literal_start;
        println!("lz4.c: start={} literals={} offset={} dup={} hash={} table={} ", literal_start, literal_len, offset, matching_bytes + backtrack, hash5(&input[literal_end..]), table.get(&input[literal_end..]));
*/

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
            step = searchMatchNb >> LZ4_skipTrigger;

// the first byte of each iteration doesn't count due to some weird-ass manual loop unrolling in the C code
if literal_start+1 != cursor {
searchMatchNb += 1
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

