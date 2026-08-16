use arbitrary_int::prelude::*;
use std::{
    fmt::Display,
    ops::Not,
    sync::atomic::{AtomicU64, Ordering},
};

#[repr(transparent)]
pub struct Metadata {
    word: AtomicU64,
}

pub enum MetadataEnum {
    // While these 3, GC calculate it itself. as this
    // encodes the size directly. The content is ignored
    PlainOldData(u61),

    // 60-bit payload for user of the GC, to determine
    // what data in an object
    NotPlainOldData(u61),
}

// Neither is true nor false as bit can be flipped.
// Meaning of Bit0 might be true for one part and other part
// means false.
//
// Internally Bit1 is treated as true for purpose of
// encoding it to compressed metadata
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Bit {
    Bit0,
    Bit1,
}

impl Display for Bit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Bit::Bit0 => write!(f, "Bit0"),
            Bit::Bit1 => write!(f, "Bit1"),
        }
    }
}

impl Not for Bit {
    type Output = Bit;

    fn not(self) -> Self::Output {
        match self {
            Bit::Bit0 => Bit::Bit1,
            Bit::Bit1 => Bit::Bit0,
        }
    }
}

pub struct MetadataExpanded {
    pub is_marked: Bit,
    pub write_barrier_activated: bool,
    pub payload: MetadataEnum,
}

/*
Bottom 4 bits. so only 60 bits payload available
0b00x => Mark bit
0bx00 => Object type
0b0x0 => write barrier already touched
*/

const MARK_BIT: u64 = 0b001;
const WRITE_BARRIER_BIT: u64 = 0b010;
const OBJECT_TYPE_MASK: u64 = 0b100;
const OBJECT_TYPE_SHIFT: u64 = 2;
const PAYLOAD_SHIFT: u64 = 3;

impl Metadata {
    pub fn new(data: MetadataExpanded) -> Metadata {
        Metadata {
            word: AtomicU64::new(Self::encode_word(data)),
        }
    }

    pub fn try_update<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        mut func: F,
    ) -> Result<MetadataExpanded, MetadataExpanded>
    where
        F: FnMut(MetadataExpanded) -> Option<MetadataExpanded>,
    {
        self.word
            .try_update(set_order, fetch_order, |x| {
                Some(Self::encode_word(func(self.decode(x))?))
            })
            .map(|x| self.decode(x))
            .map_err(|x| self.decode(x))
    }

    fn encode_word(data: MetadataExpanded) -> u64 {
        let mut v = match data.payload {
            MetadataEnum::PlainOldData(len) => (len.value() << PAYLOAD_SHIFT) | 0b000,
            MetadataEnum::NotPlainOldData(desc) => (desc.value() << PAYLOAD_SHIFT) | 0b100,
        };

        if data.is_marked == Bit::Bit1 {
            v |= MARK_BIT;
        }

        if data.write_barrier_activated {
            v |= WRITE_BARRIER_BIT;
        }

        v
    }

    fn decode(&self, v: u64) -> MetadataExpanded {
        let kind = v & OBJECT_TYPE_MASK;
        let payload = u61::extract_u64(v, PAYLOAD_SHIFT as usize);

        let payload = match kind >> OBJECT_TYPE_SHIFT {
            0b0 => MetadataEnum::PlainOldData(payload),
            0b1 => MetadataEnum::NotPlainOldData(payload),

            // This however can be the "descriptor" type one day
            // to store descriptor itself
            _ => unimplemented!("unknown type"),
        };

        MetadataExpanded {
            is_marked: if (v & MARK_BIT) != 0 {
                Bit::Bit1
            } else {
                Bit::Bit0
            },
            write_barrier_activated: (v & WRITE_BARRIER_BIT) != 0,
            payload,
        }
    }

    pub fn get(&self) -> MetadataExpanded {
        self.decode(self.word.load(Ordering::Relaxed))
    }
}
