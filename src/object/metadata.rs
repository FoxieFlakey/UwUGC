use arbitrary_int::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};

#[repr(transparent)]
pub struct Metadata {
    word: AtomicU64,
}

pub enum MetadataEnum {
    // While these 3, GC calculate it itself. as this
    // encodes the size directly. The content is ignored
    PlainOldData(u60),

    // 60-bit payload for user of the GC, to determine
    // what data in an object
    NotPlainOldData(u60),
    RefArray(u60),
}

pub struct MetadataExpanded {
    pub is_marked: bool,
    pub write_barrier_activated: bool,
    pub payload: MetadataEnum,
}

/*
Bottom 4 bits. so only 60 bits payload available
0b000x => Mark bit
0bxx00 => Object type
0b00x0 => write barrier already touched
*/

const MARK_BIT: u64 = 0b0001;
const WRITE_BARRIER_BIT: u64 = 0b0010;
const OBJECT_TYPE_MASK: u64 = 0b1100;

impl Metadata {
    pub fn new(data: MetadataExpanded) -> Metadata {
        Metadata {
            word: AtomicU64::new(Self::encode(data)),
        }
    }

    #[expect(unused)]
    pub fn update<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        mut func: F,
    ) -> MetadataExpanded
    where
        F: FnMut(MetadataExpanded) -> MetadataExpanded,
    {
        let ret = self.word.update(set_order, fetch_order, |x| {
            Self::encode(func(Self::decode(x)))
        });

        Self::decode(ret)
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
                Some(Self::encode(func(Self::decode(x))?))
            })
            .map(Self::decode)
            .map_err(Self::decode)
    }

    fn encode(data: MetadataExpanded) -> u64 {
        let mut v = match data.payload {
            MetadataEnum::PlainOldData(len) => (len.value() << 4) | 0b000,

            MetadataEnum::NotPlainOldData(desc) => (desc.value() << 4) | 0b010,

            MetadataEnum::RefArray(len) => (len.value() << 4) | 0b100,
        };

        if data.is_marked {
            v |= MARK_BIT;
        }

        if data.write_barrier_activated {
            v |= WRITE_BARRIER_BIT;
        }

        v
    }

    fn decode(v: u64) -> MetadataExpanded {
        let kind = v & OBJECT_TYPE_MASK;
        let payload = u60::extract_u64(v, 4);

        let payload = match kind {
            0b000 | 0b001 => MetadataEnum::PlainOldData(payload),
            0b010 | 0b011 => MetadataEnum::NotPlainOldData(payload),
            0b100 | 0b101 => MetadataEnum::RefArray(payload),

            // This however can be the "descriptor" type one day
            // to store descriptor itself
            _ => unimplemented!("unknown type"),
        };

        MetadataExpanded {
            is_marked: (v & MARK_BIT) != 0,
            write_barrier_activated: (v & WRITE_BARRIER_BIT) != 0,
            payload,
        }
    }

    pub fn get(&self) -> MetadataExpanded {
        Self::decode(self.word.load(Ordering::Relaxed))
    }
}
