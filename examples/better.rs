use lz_fear::compress::{compress2,U32Table,U16Table};
use lz_fear::CompressionSettings;
use std::fs::File;
use std::io::{self, Write, Read, ErrorKind};
use std::env;
use std::{cmp, mem};
use twox_hash::XxHash32;
use std::hash::Hasher;

use byteorder::{WriteBytesExt, LE};

/*
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
*/

fn main() -> io::Result<()> {
    let filename_in = env::args().skip(1).next().unwrap();
    let filename_out = env::args().skip(2).next().unwrap();
    let mut file_in = File::open(filename_in)?;
    let mut file_out = File::create(filename_out)?;
    
    CompressionSettings::default().content_checksum(true).independent_blocks(true)/*.dictionary(0, &vec![0u8; 64 * 1024])*/.dictionary_id_nonsense_override(Some(42)).compress_with_size(file_in, file_out)?;

/*
    let mut buf = Vec::new();
    file_in.read_to_end(&mut buf)?;

let flags = lz_fear::Flags2::IndependentBlocks;
let version = 1 << 6;
let flag_byte = version | flags.bits();
let bd_byte = lz_fear::Bd2::new(4 * 1024 * 1024).0;

let mut header = Vec::new();
    header.write_u32::<LE>(0x184D2204)?;
    header.write_u8(flag_byte)?;
    header.write_u8(bd_byte)?;
        /*
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
        */
        let mut hasher = XxHash32::with_seed(0);
        hasher.write(&header[4..]);
    header.write_u8((hasher.finish() >> 8) as u8)?;
    file_out.write_all(&header)?;
    
/*
    let mut buff = vec![0u8; 5 * 1024 * 1024];
    let source = buf.as_slice();
    for chunk in source.chunks(4*1024*1024) {
        compress2::<_, U32Table>(chunk, TrustMeThisIsEnoughBuffer(buff.as_mut_slice()))?;
//        file_out.write_u32::<LE>(compressed2.len() as u32)?;
        file_out.write_all(&buff)?;
    }
    file_out.write_u32::<LE>(0)?;
    
/ */
    let mut compressed2 = vec![0u8; 4 * 1024 * 1024];
    let source = buf.as_slice();
    for chunk in source.chunks(4*1024*1024) {
        let mut cursor = NoPartialWrites(&mut compressed2[..chunk.len()]); // limit output by input size so we never have negative compression ratio
        match compress2::<_, U32Table>(chunk, &mut cursor) {
            Ok(()) => {
        let not_written_len = cursor.0.len();
        let written_len = compressed2.len() - not_written_len;
        file_out.write_u32::<LE>(written_len as u32)?;
        file_out.write_all(&compressed2[..written_len])?;
            }
            Err(e) => {
                assert!(e.kind() == ErrorKind::ConnectionAborted);
                // incompressible
        file_out.write_u32::<LE>((chunk.len() as u32) | (1 << 31))?;
        file_out.write_all(chunk)?;
            }
        }
//        compressed2.clear();
    }
    file_out.write_u32::<LE>(0)?;
//*/

//    assert!(compressed.iter().eq(compressed2.iter()));

//    file_out.write_all(&compressed2)?;
//    assert!(lz4_compression::decompress::decompress(&compressed).unwrap().iter().copied().eq(buf));

/*
let mut buf = Vec::with_capacity(4 * 1024 * 1024);
while file_in.read(&mut buf)? > 0 {
    compress(&buf);
    buf.clear();
}
*/
*/
*/

    Ok(())
}
