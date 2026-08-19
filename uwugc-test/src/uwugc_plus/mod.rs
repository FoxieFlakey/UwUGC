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

    let list = uwugc_plus::alloc(&mut SafepointArgs::default(), 0, || SinglyLinked {
        data: 19,
        next: GCBoxOption::none(),
    })
    .unwrap();

    let mut list = uwugc_plus::alloc(&mut SafepointArgs::default(), 0, || SinglyLinked {
        data: 38,
        // SAFETY: We're indeed assigning here and won't dangle the pointer
        next: unsafe { GCBoxOption::new(Some(list)) },
    })
    .unwrap();

    let mut safepoint = SafepointArgs {
        state: &mut list,
        before_safepoint: |x| RootRef::store(x),
        after_safepoint: |x| RootRef::load(x),
    };

    for _ in 0..200000 {
        let root_ref: uwugc_plus::RootRef<[i32]> =
            RootRef::coerce(uwugc_plus::alloc(&mut safepoint, 0, || [0; 16 * 1024]).unwrap());
        assert_eq!(root_ref.len(), 16 * 1024, "aaa");
        drop(root_ref);

        uwugc_plus::safepoint(&mut safepoint);
    }

    println!("A: {}", list.data);

    let list = list.next.load().unwrap();
    println!("B: {}", list.data);
}
