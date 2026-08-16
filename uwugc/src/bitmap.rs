use std::sync::atomic::{AtomicUsize, Ordering};

pub struct AtomicBitmap {
    mem: Vec<AtomicUsize>,
    len: usize,
}

impl AtomicBitmap {
    pub fn new(len: usize) -> Self {
        let len_words = len.div_ceil(usize::BITS.try_into().unwrap());

        let mut mem = Vec::new();
        mem.resize_with(len_words, Default::default);
        Self { mem, len }
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
        if idx >= self.len {
            panic!("out of index bound, len is {} but index is {idx}", self.len);
        }

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
