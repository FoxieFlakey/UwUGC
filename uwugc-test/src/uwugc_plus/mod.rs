use std::{marker::PhantomData, sync::atomic::Ordering};

use uwugc::UwUGC;
use uwugc_plus::{GCBoxOption, HasDescriptor, RootRef, SafepointArgs, UwUGCPlus};

// Spamming garbage objects test

#[derive(HasDescriptor)]
pub struct SinglyLinked {
    next: GCBoxOption<SinglyLinked>,
    data: u32,
}

pub fn run(uwugc: UwUGC) {
    let uwugc = UwUGCPlus::new(uwugc);
    uwugc.init_context();

    let mut safepoint = SafepointArgs::default();
    for _ in 0..200000 {
        let root_ref: uwugc_plus::RootRef<[i32]> =
            uwugc_plus::alloc(&mut safepoint, || [0; 16 * 1024]).unwrap();
        assert_eq!(root_ref.len(), 16 * 1024, "aaa");
        drop(root_ref);

        uwugc_plus::safepoint(&mut safepoint);
    }
}
