use uwugc::UwUGC;
use uwugc_plus::{SafepointArgs, UwUGCPlus};

// Spamming garbage objects test

pub fn run(uwugc: UwUGC) {
    let uwugc = UwUGCPlus::new(uwugc);
    uwugc.init_context();

    let mut safepoint = SafepointArgs::default();
    for _ in 0..200000 {
        uwugc_plus::alloc(&mut safepoint, || [0; 16 * 1024]);
        uwugc_plus::safepoint(&mut safepoint);
    }
}
