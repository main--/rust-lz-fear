#![allow(non_upper_case_globals)]

use std::fmt::Debug;
use thiserror::Error;
use fehler::{throw, throws};
use bitflags::bitflags;

bitflags! {
    pub struct Flags: u8 {
        const IndependentBlocks = 0b00100000;
        const BlockChecksums    = 0b00010000;
        const ContentSize       = 0b00001000;
        const ContentChecksum   = 0b00000100;
        const DictionaryId      = 0b00000001;
    }
}

#[derive(Error, Debug)]
pub enum ParseError {
    #[error("at the time of writing this, spec says value {0} is reserved")]
    UnimplementedBlocksize(u8),
    #[error("file version {0} not supported")]
    UnsupportedVersion(u8),
    #[error("reserved bits in flags set")]
    ReservedFlagBitsSet,
    #[error("reserved bits in bd set")]
    ReservedBdBitsSet,
}

impl Flags {
    #[throws(ParseError)]
    pub fn parse(i: u8) -> Self {
        let version = i >> 6;
        if version != 1 {
            throw!(ParseError::UnsupportedVersion(version));
        }
        if (i & 0b10) != 0 {
            throw!(ParseError::ReservedFlagBitsSet);
        }

        Flags::from_bits_truncate(i)
    }

    pub fn independent_blocks(&self) -> bool { self.contains(Flags::IndependentBlocks) }
    pub fn block_checksums(&self)    -> bool { self.contains(Flags::BlockChecksums) }
    pub fn content_size(&self)       -> bool { self.contains(Flags::ContentSize) }
    pub fn content_checksum(&self)   -> bool { self.contains(Flags::ContentChecksum) }
    pub fn dictionary_id(&self)      -> bool { self.contains(Flags::DictionaryId) }
}

pub struct BlockDescriptor(pub u8); // ??? or what else could "BD" stand for ???
impl BlockDescriptor {
//    #[throws]
    pub fn new(block_maxsize: usize) -> Self {
        let maybe_maxsize = ((block_maxsize.trailing_zeros().saturating_sub(8)) / 2) as u8;
        let bd = BlockDescriptor::parse(maybe_maxsize << 4).unwrap();
        assert_eq!(block_maxsize, bd.block_maxsize().unwrap());

        bd
    }

    #[throws(ParseError)]
    pub fn parse(i: u8) -> Self {
        if (i & 0b10001111) != 0 {
            throw!(ParseError::ReservedBdBitsSet);
        }
        BlockDescriptor(i)
    }

    #[throws(ParseError)]
    pub fn block_maxsize(&self) -> usize {
        let size = (self.0 >> 4) & 0b111;
        if (4..8).contains(&size) {
            1 << (size * 2 + 8)
        } else {
            throw!(ParseError::UnimplementedBlocksize(size))
        }
    }
}

