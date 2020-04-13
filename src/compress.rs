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
        self.dict[hash5(key)].try_into().expect("This code is not supposed to run on a 16 arch (let alone smaller)")
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

/// A LZ4 block.
///
/// This defines a single compression "unit", consisting of two parts, a number of raw literals,
/// and possibly a pointer to the already encoded buffer from which to copy.
#[derive(Debug)]
struct Block {
    /// The length (in bytes) of the literals section.
    lit_len: usize,

    /// The duplicates section if any.
    ///
    /// Only the last block in a stream can lack of the duplicates section.
    dup: Option<Duplicate>,
}

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

/// An LZ4 encoder.
pub struct Encoder<'a, T> {
    /// The raw uncompressed input.
    input: &'a [u8],

    /// The compressed output.
    output: &'a mut Vec<u8>,

    /// The number of bytes from the input that are encoded.
    cur: usize,

    /// The dictionary of previously encoded sequences.
    ///
    /// This is used to find duplicates in the stream so they are not written multiple times.
    ///
    /// Every four bytes are hashed, and in the resulting slot their position in the input buffer
    /// is placed. This way we can easily look up a candidate to back references.
    table: T,
}

fn eq4bytes(a: &[u8], b: &[u8]) -> bool {
    NativeEndian::read_u32(a) == NativeEndian::read_u32(b)
}

impl<'a, T: EncoderTable> Encoder<'a, T> {
    /// Go forward by some number of bytes.
    ///
    /// This will update the cursor and dictionary to reflect the now processed bytes.
    ///
    /// This returns `false` if all the input bytes are processed.
    fn go_forward(&mut self, mut steps: usize, egal: bool) -> bool {
        //if steps > 1 {
        if egal {
            assert!(steps >= 2);
            self.insert_cursor();
            
            let i = 2;
            self.cur += steps - i;
            steps = i;

//            self.cur += steps - 1;
//            steps = 1;
            
            self.insert_cursor();
            self.cur += i;
            return self.cur <= self.input.len();
        }

/*
        // Go over all the bytes we are skipping and update the cursor and dictionary.
        for _ in 0..steps {
            // Insert the cursor position into the dictionary.
            self.insert_cursor();

            // Increment the cursor.
            self.cur += 1;
        }
*/
	self.insert_cursor();
	self.cur += steps;

        // Return `true` if there's more to read.
        self.cur <= self.input.len()
    }

    /// Insert the batch under the cursor into the dictionary.
    fn insert_cursor(&mut self) {
        // Make sure that there is at least one batch remaining.
        if self.remaining_batch() {
            // Insert the cursor into the table.
            
//            println!("inserting@{:04} {:016x}", self.cur, self.get_batch_at_cursor().swap_bytes());
            
            self.table.set(&self.input[self.cur..], self.cur);
//            self.dict[self.get_cur_hash()] = self.cur as u32;
        }
    }

    /// Check if there are any remaining batches.
    fn remaining_batch(&self) -> bool {
        self.cur + 4 < self.input.len()
    }

    /// Read a 4-byte "batch" from some position.
    ///
    /// This will read a native-endian 4-byte integer from some position.
    fn get_batch(&self, n: usize) -> &[u8] {
        assert!(self.remaining_batch(), "Reading a partial batch.");

	//let xo = if (n + 8) <= self.input.len() { NativeEndian::read_u32(&self.input[n+4..]) as u64 } else { 0 };
	//let xo = if (n + 5) <= self.input.len() { self.input[n+4] as u32 } else { 0 };
	//let zeroes: &[u8] = &[0; 4];
	//(&self.input[n..]).chain(zeroes).read_u64::<LE>().unwrap()
	//(&self.input[n..]).chain(zeroes).read_u32::<LE>().unwrap() as u64
	
//	let upper_byte = self.input.get(n+4).copied().unwrap_or(0);
//        NativeEndian::read_u32(&self.input[n..]) as u64 | ((upper_byte as u64) << 32)
        &self.input[n..]
    }

    /// Find a duplicate of the current batch.
    ///
    /// If any duplicate is found, a tuple `(position, size - 4)` is returned.
    fn find_duplicate(&self) -> Option<Duplicate> {
        // If there is no remaining batch, we return none.
        if !self.remaining_batch() {
            return None;
        }

        let current_batch = self.get_batch(self.cur);
        // Find a candidate in the dictionary by hashing the current four bytes.
//        let candidate = self.dict[self.get_cur_hash()] as usize;
        let candidate = self.table.get(current_batch);

        // Three requirements to the candidate exists:
        // - The candidate is not the trap value (0xFFFFFFFF), which represents an empty bucket.
        // - We should not return a position which is merely a hash collision, so w that the
        //   candidate actually matches what we search for.
        // - We can address up to 16-bit offset, hence we are only able to address the candidate if
        //   its offset is less than or equals to 0xFFFF.
        if self.cur != 0 //candidate != !0
            && eq4bytes(self.get_batch(candidate), current_batch)
//            && (self.get_batch(candidate) as u32) == (self.get_batch_at_cursor() as u32)
            && self.cur - candidate <= 0xFFFF {

            // Calculate the "extension bytes", i.e. the duplicate bytes beyond the batch. These
            // are the number of prefix bytes shared between the match and needle.
            /*
            let ext = self.input[self.cur + 4..]
                .iter()
                .zip(&self.input[candidate + 4..])
                .take_while(|&(a, b)| a == b)
                .count();
                */
                
                
                //
            let mut ext = 0;
//            for (a, b) in self.input[self.cur+4..].chunks_exact(4).zip(self.input[candidate+4..].chunks_exact(4)) {
            for (a, b) in self.input[self.cur+4..].chunks(4).zip(self.input[candidate+4..].chunks(4)) {
                // as the candidate is always behind our cursor, a is always smaller
                if a.len() != 4 /*|| b.len() != 4*/ {
                    // slow path, can't read full integers
                    ext += a.iter().zip(b).take_while(|&(a, b)| a == b).count();
                    break;
                }

// TODO: can do 64 bits here
//                println!("{:?} vs {:?}", a, b);
                let a = NativeEndian::read_u32(a);
                let b = NativeEndian::read_u32(b);
                let xor = a ^ b;
                if xor == 0 {
                    ext += 4;
//                    println!("4ext={}", ext);
                } else {
                // FIXME depends on endianness?
                    ext += xor.trailing_zeros() as usize / 8;
//                    println!("next={} by {:x}", ext, xor);
                    break;
                }
            }

            Some(Duplicate {
                offset: (self.cur - candidate) as u16,
                extra_bytes: ext,
            })
        } else { None }
    }

    /// Write an integer to the output in LSIC format.
    fn write_integer(&mut self, mut n: usize) {
        // Write the 0xFF bytes as long as the integer is higher than said value.
        while n >= 0xFF {
            n -= 0xFF;
            self.output.push(0xFF);
        }

        // Write the remaining byte.
        self.output.push(n as u8);
    }

    /// Read the block of the top of the stream.
    fn pop_block(&mut self) -> Block {
        let LZ4_skipTrigger = 6;
    let mut searchMatchNb = /*acceleration*/1 << LZ4_skipTrigger;
    let mut step = 1;
    
    
        // The length of the literals section.
        let mut lit = 0;

        loop {
            // Search for a duplicate.
            if let Some(mut dup) = self.find_duplicate() {
                // We found a duplicate, so the literals section is over...

            // backtrack
            let mut backtrack = 0;
            loop {
                if (dup.offset as usize + backtrack) == 0xffff { break; }
                if backtrack == lit { break; }
                if (self.cur - dup.offset as usize - backtrack) <= 1 { break; }
//                println!("ext {} vs {}", self.input[(self.cur - dup.offset as usize) - 1 - backtrack], self.input[self.cur - 1 - backtrack]);
                if self.input[(self.cur - dup.offset as usize) - 1 - backtrack] != self.input[self.cur - 1 - backtrack] { break; }
                backtrack += 1;
//                println!("updated to {} {}", self.cur - candidate, ext);
            }


                // Move forward. Note that `ext` is actually the steps minus 4, because of the
                // minimum matchlenght, so we need to add 4.
                self.go_forward(dup.extra_bytes + 4 /*- backtrack*/, true);
                dup.extra_bytes += backtrack;
                //dup.offset += backtrack as u16;

                return Block {
                    lit_len: lit - backtrack,
                    dup: Some(dup),
                };
            }

            // Try to move forward.
            if !self.go_forward(step, false) {
                // We reached the end of the stream, and no duplicates section follows.
                return Block {
                    lit_len: lit,
                    dup: None,
                };
            }

            // No duplicates found yet, so extend the literals section.
            lit += step;
            
            step = searchMatchNb >> LZ4_skipTrigger;
            searchMatchNb += 1;
        }
    }

    /// Complete the encoding into `self.output`.
    fn complete(&mut self) {
        // Construct one block at a time.
        loop {
            // The start of the literals section.
            let start = self.cur;

            // Read the next block into two sections, the literals and the duplicates.
            let block = self.pop_block();
//            println!("{:?}", block);

            // Generate the higher half of the token.
            let mut token = if block.lit_len < 0xF {
                // Since we can fit the literals length into it, there is no need for saturation.
                (block.lit_len as u8) << 4
            }
            else {
                // We were unable to fit the literals into it, so we saturate to 0xF. We will later
                // write the extensional value through LSIC encoding.
                0xF0
            };

            // Generate the lower half of the token, the duplicates length.
            let dup_extra_len = block.dup.map_or(0, |x| x.extra_bytes);
            token |= if dup_extra_len < 0xF {
                // We could fit it in.
                dup_extra_len as u8
            }
            else {
                // We were unable to fit it in, so we default to 0xF, which will later be extended
                // by LSIC encoding.
                0xF
            };

            // Push the token to the output stream.
            self.output.push(token);

            // If we were unable to fit the literals length into the token, write the extensional
            // part through LSIC.
            if block.lit_len >= 0xF {
                self.write_integer(block.lit_len - 0xF);
            }

            // Now, write the actual literals.
            self.output.extend_from_slice(&self.input[start..start + block.lit_len]);

            if let Some(Duplicate { offset, .. }) = block.dup {
                // Wait! There's more. Now, we encode the duplicates section.

                // Push the offset in little endian.
                self.output.push(offset as u8);
                self.output.push((offset >> 8) as u8);

                // If we were unable to fit the duplicates length into the token, write the
                // extensional part through LSIC.
                if dup_extra_len >= 0xF {
                    self.write_integer(dup_extra_len - 0xF);
                }
            } else {
                break;
            }
        }
    }
}

/// Compress all bytes of `input` into `output`.
pub fn compress_into(input: &[u8], output: &mut Vec<u8>) {
    Encoder {
        input,
        output,
        cur: 0,
//        dict: [!0; DICTIONARY_SIZE],
        table: U32Table::default(), //[0; DICTIONARY_SIZE],
    }.complete();
}


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
                writer.write_u8(token);
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
            step = searchMatchNb >> LZ4_skipTrigger;
            searchMatchNb += 1;
        };
        
        // cursor is now pointing past the match
        let literal_end = cursor - duplicate.extra_bytes - MINMATCH;
        let literal_len = literal_end - literal_start;
        
//        println!("loopy {} {:?}", literal_len, duplicate);
        
        let mut token = 0;
        write_lsic_head(&mut token, 4, literal_len);
        write_lsic_head(&mut token, 0, duplicate.extra_bytes);

        writer.write_u8(token);
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

    while value >= 0xFF {
        writer.write_u8(0xFF);
        value -= 0xFF;
    }
    writer.write_u8(value as u8);
}


/// Compress all bytes of `input`.
pub fn compress(input: &[u8]) -> Vec<u8> {
    // In most cases, the compression won't expand the size, so we set the input size as capacity.
    let mut vec = Vec::with_capacity(input.len());

    compress_into(input, &mut vec);

    vec
}
