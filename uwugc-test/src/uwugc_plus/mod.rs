use uwugc::UwUGC;
use uwugc_plus::{GCBoxOption, HasDescriptor, RootRef, Safepoint, UwUGCPlus};

// Spamming garbage objects test

#[derive(HasDescriptor)]
pub struct SinglyLinked {
    next: GCBoxOption<SinglyLinked>,
    data: u32,
}

// A safepoint structure mainly used for if caller
// want to store/load some root refs.
pub struct SafepointArgs<T, F1, F2>
where
    F1: FnMut(&mut T),
    F2: FnMut(&mut T),
{
    pub before_safepoint: F1,
    pub after_safepoint: F2,
    pub state: T,
}

impl Default for SafepointArgs<(), fn(&mut ()), fn(&mut ())> {
    fn default() -> Self {
        Self {
            before_safepoint: |_| (),
            after_safepoint: |_| (),
            state: (),
        }
    }
}

impl<S, F1, F2> Safepoint for SafepointArgs<S, F1, F2>
    where F1: FnMut(&mut S),
        F2: FnMut(&mut S),
{
    fn after_safepoint(&mut self) {
        (self.after_safepoint)(&mut self.state)
    }

    fn before_safepoint(&mut self) {
        (self.before_safepoint)(&mut self.state)
    }
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

    let list = list.next.get_ref().unwrap();
    println!("B: {}", list.data);
}
