use uwugc::UwUGC;
use uwugc_plus::{GCBoxOption, RootRef, SafepointList, SafepointMut, UwUGCPlus, safe_roots};
use uwugc_plus_alloc::Vec;

// Ported from
// https://github.com/WillSewell/gc-latency-experiment/blob/f67121ec8a741201414c76d5ba85f9304c774acc/java/Main.java

const WINDOW_SIZE: usize = 200_000;
const MSG_COUNT: usize = 1_000_000;
const MSG_SIZE: usize = 1024;

fn create_message(mut safepoint: &mut dyn SafepointMut, n: usize) -> RootRef<[u8; MSG_SIZE]> {
    uwugc_plus::alloc(&mut safepoint, 0, || [(n % 0xFF) as u8; MSG_SIZE]).expect("Out of memory")
}

fn push_message(mut safepoint: &mut dyn SafepointMut, store: &mut RootRef<Vec<GCBoxOption<[u8; MSG_SIZE]>>>, id: usize) {
    let allocated = create_message(&mut SafepointList(&mut [ &mut safepoint, &mut &store ]), id);
    store[id % WINDOW_SIZE].store(Some(allocated));
}

#[expect(unused)]
pub fn run(uwugc: UwUGC) {
    // SAFETY: Nah we dont do illegals
    let uwugc = unsafe { UwUGCPlus::new(uwugc) };
    uwugc.init_context();

    let mut store = Vec::new(&mut safe_roots!()).expect("Out of memory");
    store.resize_with(&mut safe_roots!(), WINDOW_SIZE, || GCBoxOption::none()).expect("Out of memory");

    for i in 0..MSG_COUNT {
        push_message(&mut safe_roots!(), &mut store, i);
    }
}


