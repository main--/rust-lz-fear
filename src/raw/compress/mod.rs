use std::mem;
use std::cmp;
use std::io::Write;
use std::convert::{TryInto, TryFrom};
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


pub trait EncoderTable {
    fn payload_size_limit() -> usize;
    // offset is declared as usize but must not be above payload_size_limit
    fn replace(&mut self, input: &[u8], offset: usize) -> usize;

    fn offset(&mut self, offset: usize);
}

#[derive(Clone)]
pub struct U32Table {
    dict: [u32; DICTIONARY_SIZE],
    offset: usize,
}
impl Default for U32Table {
    fn default() -> Self {
        U32Table { dict: [0; DICTIONARY_SIZE], offset: 0 }
    }
}


// on 64 bit systems, we read 64 bits and hash 5 bytes instead of 4
#[cfg(target_pointer_width = "64")]
fn hash_for_u32(input: &[u8]) -> usize {
    // read 64 bits if possible
    let v = input.get(..8).map(NativeEndian::read_u64).unwrap_or(0);
    // we end up only needing 5 bytes but the only case where this becomes
    // zero is at the very end, where we're not allowed to produce matches anyway (see below)

    // calculate a bad but very cheap checksum
    #[cfg(target_endian = "little")] fn checksum_u64(v: u64) -> u64 { (v << 24).wrapping_mul(889523592379) }
    #[cfg(target_endian = "big")] fn checksum_u64(v: u64) -> u64 { (v >> 24).wrapping_mul(11400714785074694791) }
    (checksum_u64(v) >> (64 - HASHLOG)) as usize
}
// on all other systems we simply hash 4 bytes, borrowing the algorithm for the u16 table
#[cfg(not(target_pointer_width = "64"))]
fn hash_for_u32(input: &[u8]) -> usize {
    hash_for_u16(input) >> 1 // shift by one more because we have half as many slots as the u16 table
}

fn hash_for_u16(input: &[u8]) -> usize {
    let v = NativeEndian::read_u32(input);
    (v.wrapping_mul(2654435761) >> (32 - HASHLOG - 1)) as usize // shift by one less than hashlog because we have twice as many slots
}

impl EncoderTable for U32Table {
    fn replace(&mut self, input: &[u8], offset: usize) -> usize {
        let o = offset + self.offset; // apply positive offset on input

        let mut value = o.try_into().expect("EncoderTable contract violated");
        mem::swap(&mut self.dict[hash_for_u32(&input[offset..])], &mut value);
        usize::try_from(value).expect("This code is not supposed to run on a 16-bit arch (let alone smaller)")
            .saturating_sub(self.offset) // apply negative offset on output
    }
    fn offset(&mut self, offset: usize) {
        self.offset += offset;
    }
    fn payload_size_limit() -> usize { std::u32::MAX as usize }
}

#[derive(Clone)]
pub struct U16Table {
    dict: [u16; DICTIONARY_SIZE*2], // u16 fits twice as many slots into the same amount of memory
    offset: usize,
}
impl Default for U16Table {
    fn default() -> Self {
        U16Table { dict: [0; DICTIONARY_SIZE*2], offset: 0 }
    }
}
impl EncoderTable for U16Table {
    fn replace(&mut self, input: &[u8], offset: usize) -> usize {
        let o = offset + self.offset; // apply positive offset on input

        let mut value = o.try_into().expect("EncoderTable contract violated");
        mem::swap(&mut self.dict[hash_for_u16(&input[offset..])], &mut value);
        usize::try_from(value).expect("This code is not supposed to run on a 16-bit arch (let alone smaller)")
            .saturating_sub(self.offset) // apply negative offset on output
    }
    fn offset(&mut self, offset: usize) {
        self.offset += offset;
    }
    fn payload_size_limit() -> usize { std::u16::MAX as usize }
}


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
fn write_group<W: Write>(mut writer: &mut W, literal: &[u8], duplicate: Duplicate) {
        let literal_len = literal.len();

        let mut token = 0;
        write_lsic_head(&mut token, 4, literal_len);
        write_lsic_head(&mut token, 0, duplicate.extra_bytes);

        writer.write_u8(token)?;
        write_lsic_tail(&mut writer, literal_len)?;
        writer.write_all(literal)?;
        writer.write_u16::<LE>(duplicate.offset)?;
        write_lsic_tail(&mut writer, duplicate.extra_bytes)?;
}

#[throws]
pub fn compress2<W: Write, T: EncoderTable>(input: &[u8], cursor: usize, table: &mut T, mut writer: W) {
    assert!(input.len() <= T::payload_size_limit());

    let init_cursor = cursor;
    let mut cursor = cursor;
    while cursor < input.len() {
        let literal_start = cursor;

        let mut step_counter = ACCELERATION << SKIP_TRIGGER;
        let mut step = 1;
        // look for a duplicate
        let duplicate = loop {
            if input.len().saturating_sub(cursor) < 12 {
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
            let candidate = table.replace(input, cursor);

            // NB: for correctness, only comparing to 0 is needed here (gives better compression ratio when using dependent blocks)
            //     however the reference implementation strictly enforces this and we strive for byte-perfect output
            if (cursor != init_cursor) // can never match on the very first byte
                && cursor - candidate <= 0xFFFF { // must be an addressable offset
                // let's see how many matching bytes we have
                let candidate_batch = &input[candidate..];
                let matching_bytes = count_matching_bytes(current_batch, candidate_batch);

                if let Some(mut extra_bytes) = matching_bytes.checked_sub(MINMATCH) {
                    // if it wasn't, this was just a hash collision :(
                    let offset = (cursor - candidate) as u16;

                    // backtrack
                    let max_backtrack = cursor - literal_start;
                    let backtrack = input[..cursor].iter().rev().zip(input[..candidate].iter().rev()).take(max_backtrack).take_while(|&(a, b)| a == b).count();
                    // offset remains unchanged
                    extra_bytes += backtrack;
                    cursor += matching_bytes;

                    // not sure why exactly cursor - 2, but that's what they do
                    table.replace(input, cursor - 2);
        
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
        write_group(&mut writer, &input[literal_start..literal_end], duplicate)?;
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

    while value >= 4 * 0xFF {
        writer.write_u32::<NativeEndian>(std::u32::MAX)?;
        value -= 4 * 0xFF;
    }
    while value >= 0xFF {
        writer.write_u8(0xFF)?;
        value -= 0xFF;
    }
    writer.write_u8(value as u8)?;
}

