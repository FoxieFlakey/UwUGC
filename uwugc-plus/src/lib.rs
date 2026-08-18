#![feature(unsize)]
#![feature(coerce_unsized)]
#![feature(map_try_insert)]
#![feature(set_ptr_value)]

// An high level interface for uwugc with extra structures and alike.
// inter mixing calls thru this and direct calls are very fragile and
// be done with care.

use std::{marker::PhantomData, mem::MaybeUninit, sync::Arc};

use uwugc::UwUGC;
use yoke::Yoke;

use crate::{
    context::Context,
    types::{TypeId, Types},
};

mod context;
mod gcref;
mod has_descriptor;
mod typed_root_ref;

mod types;

pub struct UwUGCPlus(UwUGC);
pub use gcref::{GCBox, GCBoxOption};
pub use has_descriptor::HasDescriptor;
pub use typed_root_ref::RootRef;
pub use types::Descriptor;

impl UwUGCPlus {
    // Note: passing UwUGC to here, will
    // replaces TypeManager that is in there
    pub fn new(mut uwugc: UwUGC) -> Arc<UwUGCPlus> {
        uwugc.set_type_manager(Types::new());
        Arc::new(UwUGCPlus(uwugc))
    }

    // Init context on current thread
    pub fn init_context(self: &Arc<UwUGCPlus>) {
        context::CURRENT_CONTEXT.with_borrow_mut(|x| {
            assert!(
                x.is_none(),
                "Cannot have multiple uwugcplus context on one thread"
            );
            *x = Some(Yoke::attach_to_cart(self.clone(), |x| Context::new(&x.0)));
        });
    }
}

// A safepoint structure mainly used for if caller
// want to store/load some root refs.
pub struct SafepointArgs<'a, F1, F2>
where
    F1: FnMut() + 'a,
    F2: FnMut() + 'a,
{
    pub before_safepoint: F1,
    pub after_safepoint: F2,
    pub phantom: PhantomData<&'a ()>,
}

impl Default for SafepointArgs<'_, fn(), fn()> {
    fn default() -> Self {
        Self {
            before_safepoint: || (),
            after_safepoint: || (),
            phantom: PhantomData,
        }
    }
}

// Free standing functions
pub fn safepoint<'a, F1, F2>(args: &mut SafepointArgs<'a, F1, F2>)
where
    F1: FnMut() + 'a,
    F2: FnMut() + 'a,
{
    (args.before_safepoint)();
    context::with_context_mut(|x| x.safepoint());
    (args.after_safepoint)();
}

pub fn alloc<'a, T, F, F1, F2>(
    safepoint_args: &mut SafepointArgs<'a, F1, F2>,
    init: F,
) -> Option<RootRef<T>>
where
    F1: FnMut() + 'a,
    F2: FnMut() + 'a,
    F: FnOnce() -> T + 'a,
    T: HasDescriptor,
{
    context::with_context_mut(|x| x.alloc_fast(TypeId::from(T::DESCRIPTOR)))
        .or_else(|| {
            (safepoint_args.before_safepoint)();
            let ret = context::with_context_mut(|x| x.alloc_slow(TypeId::from(T::DESCRIPTOR)));
            (safepoint_args.after_safepoint)();
            ret
        })
        .map(|x| {
            let mut ptr = x.get_ptr().data().cast::<MaybeUninit<T>>();
            // SAFETY: Allocator allocated correct sizing and stuffs, so its safe to write
            unsafe { ptr.as_mut() }.write(init());

            // SAFETY: We allocated with correct descriptor for given type
            // by constructing Descriptor, caller guarantee its correct. So
            // we trust it
            unsafe { RootRef::from_raw(x) }
        })
}
