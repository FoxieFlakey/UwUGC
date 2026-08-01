use std::sync::atomic::{AtomicUsize, Ordering};

pub struct AtomicBitmap {
    mem: Vec<AtomicUsize>,
    len: usize,
}

#[expect(unused)]
impl AtomicBitmap {
    pub fn new(mut len: usize) -> Self {
        len = len.div_ceil(usize::BITS.try_into().unwrap());

        let mut mem = Vec::new();
        mem.resize_with(len, Default::default);
        Self { mem, len }
    }

    pub fn get(&self, idx: usize, order: Ordering) -> bool {
        if idx > self.len {}

        let (word, mask) = Self::calc_bit_pos(idx);
        self.mem[word].load(order) & mask != 0
    }

    // Like other operations in Atomic that is RMW.
    // These are actual operations
    //
    // Order    | Fetch order   | Set order
    // --------------------------------------
    // Acquire  | Acquire       | Relaxed
    // Release  | Relaxed       | Release
    // AcqRel   | Acquire       | Release
    // SeqCst   | SeqCst        | SeqCst
    // Relaxed  | Relaxed       | Relaxed
    pub fn set(&self, idx: usize, value: bool, order: Ordering) -> bool {
        let (fetch_order, set_order) = Self::make_fetch_store_order(order);

        self.update(set_order, fetch_order, idx, |_| value)
    }

    pub fn toggle(&self, idx: usize, value: bool, order: Ordering) -> bool {
        let (fetch_order, set_order) = Self::make_fetch_store_order(order);
        self.update(set_order, fetch_order, idx, |x| !x)
    }

    pub fn update<F>(
        &self,
        set_order: Ordering,
        fetch_order: Ordering,
        idx: usize,
        mut func: F,
    ) -> bool
    where
        F: FnMut(bool) -> bool,
    {
        let (word, mask) = Self::calc_bit_pos(idx);
        self.mem[word].update(set_order, fetch_order, |x| {
            if func(x & mask != 0) {
                x | mask
            } else {
                x & (!mask)
            }
        }) & mask
            != 0
    }

    // Iterate the bitmap. Bit by bit. Note. other thread might
    // modify the bitmap concurrently. This boiled down to be
    // naive for each then atomic load. For example
    //
    // for (idx, bit) in bitmap.iter(Ordering::Relaxed) {
    //     if bit {
    //         println!("hi! at {idx}");
    //     }
    // }
    //
    // Wouldnt work not prevent other thread modifying afer you get the bit. More less similar
    // to "data race" depends on what you doing. Atomic only prevents half written and reordering
    // on access to it. Just like typical atomic loads. It takes a snapshot at that point. but
    // value is definitely 100% outdated in presence of other threads modifying.
    //
    // order is ordering used for get
    pub fn iter<'a>(&'a self, order: Ordering) -> Iter<'a> {
        Iter {
            current: 0,
            map: self,
            order,
        }
    }

    fn make_fetch_store_order(order: Ordering) -> (Ordering, Ordering) {
        match order {
            Ordering::Acquire => (Ordering::Acquire, Ordering::Relaxed),
            Ordering::Release => (Ordering::Relaxed, Ordering::Release),
            Ordering::AcqRel => (Ordering::Acquire, Ordering::Release),
            Ordering::SeqCst => (Ordering::SeqCst, Ordering::SeqCst),
            Ordering::Relaxed => (Ordering::Relaxed, Ordering::Relaxed),

            _ => unimplemented!(),
        }
    }

    fn calc_bit_pos(index: usize) -> (usize, usize) {
        (
            index / usize::try_from(usize::BITS).unwrap(),
            1 << u32::try_from(index % usize::try_from(usize::BITS).unwrap()).unwrap(),
        )
    }
}

pub struct Iter<'a> {
    map: &'a AtomicBitmap,
    order: Ordering,
    current: usize,
}

impl Iterator for Iter<'_> {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current > self.map.len {
            None
        } else {
            let ret = self.map.get(self.current, self.order);
            self.current += 1;
            Some(ret)
        }
    }
}

impl IntoIterator for AtomicBitmap {
    type Item = bool;
    type IntoIter = IntoIter;

    fn into_iter(self) -> Self::IntoIter {
        IntoIter {
            map: self,
            current: 0,
        }
    }
}

// Ordering is Relaxed. Because only current thread has this instance
// if ordering necessary to access other data. Please put necessary
// atomic barriers/fences
pub struct IntoIter {
    map: AtomicBitmap,
    current: usize,
}

impl Iterator for IntoIter {
    type Item = bool;

    fn next(&mut self) -> Option<Self::Item> {
        if self.current > self.map.len {
            None
        } else {
            let ret = self.map.get(self.current, Ordering::Relaxed);
            self.current += 1;
            Some(ret)
        }
    }
}
