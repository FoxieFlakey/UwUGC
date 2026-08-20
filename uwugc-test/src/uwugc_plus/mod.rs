use uwugc::UwUGC;
use uwugc_plus::{GCBoxOption, HasDescriptor, RootRef, UwUGCPlus, safe_roots};

// Spamming garbage objects test

#[derive(HasDescriptor)]
pub struct SinglyLinked {
    next: GCBoxOption<SinglyLinked>,
    data: u32,
}

pub fn run(uwugc: UwUGC) {
    let uwugc = UwUGCPlus::new(uwugc);
    uwugc.init_context();

    let list = uwugc_plus::alloc(&mut [], 0, || SinglyLinked {
        data: 19,
        next: GCBoxOption::none(),
    })
    .unwrap();

    let list = uwugc_plus::alloc(&mut [], 0, || SinglyLinked {
        data: 38,
        // SAFETY: We're indeed assigning here and won't dangle the pointer
        next: unsafe { GCBoxOption::new(Some(list)) },
    })
    .unwrap();

    let mut safepoint = safe_roots!(&list);
    let mut vec: RootRef<uwugc_plus_alloc::Vec<GCBoxOption<SinglyLinked>>> =
        uwugc_plus_alloc::Vec::new(&mut safepoint).unwrap();

    vec.insert(&mut safepoint, GCBoxOption::none()).unwrap();
    vec[0].store(Some(list));

    let mut safepoint = safe_roots!(&vec);
    for _ in 0..200000 {
        let root_ref: uwugc_plus::RootRef<[i32]> =
            RootRef::coerce(uwugc_plus::alloc(&mut safepoint, 0, || [0; 16 * 1024]).unwrap());
        assert_eq!(root_ref.len(), 16 * 1024, "aaa");
        drop(root_ref);

        uwugc_plus::safepoint(&mut safepoint);
    }

    let list = vec[0].get_ref().unwrap();
    println!("A: {}", list.data);

    let list = list.next.get_ref().unwrap();
    println!("B: {}", list.data);
}
