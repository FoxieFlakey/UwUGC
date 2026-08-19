#![feature(ptr_metadata)]
#![feature(unsize)]
#![feature(map_try_insert)]

// An high level interface for uwugc with extra structures and alike.
// inter mixing calls thru this and direct calls are very fragile and
// be done with care.

use std::{marker::PhantomData, mem::MaybeUninit, sync::Arc};

use uwugc::UwUGC;
use yoke::Yoke;

use crate::{
    array_metadata::ArrayHeader,
    context::Context,
    types::{TypeId, Types},
};

mod array;
mod array_metadata;
mod context;
mod descriptor;
mod gcref;
mod has_descriptor;
mod typed_root_ref;
mod types;

pub struct UwUGCPlus(UwUGC);
pub use array::Array;
pub use descriptor::Descriptor;
pub use gcref::{GCBox, GCBoxOption};
pub use has_descriptor::HasDescriptor;
pub use typed_root_ref::RootRef;
pub use context::RootRefRaw;

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

pub trait Safepoint {
    fn before_safepoint(&mut self);
    fn after_safepoint(&mut self);
}

impl<'a, T> Safepoint for T
    where T: AsRef<[&'a RootRefRaw]>
{
    fn before_safepoint(&mut self) {
        self.as_ref()
            .iter()
            .for_each(|x| {
                x.store();
            });
    }

    fn after_safepoint(&mut self) {
        self.as_ref()
            .iter()
            .for_each(|x| {
                x.load();
            });
    }
}

// Convenient macro for saving/loading root refs
#[macro_export]
macro_rules! safe_roots {
    ($($item:expr),* $(,)?) => {
        [
            $(
                $crate::RootRef::as_raw($item)
            )*
        ]
    };
}

// Free standing functions
pub fn safepoint(safepoint: &mut dyn Safepoint) {
    safepoint.before_safepoint();
    context::with_context_mut(|x| x.safepoint());
    safepoint.after_safepoint();
}

pub fn alloc_array<'a, T, F>(
    safepoint: &mut dyn Safepoint,
    extra_bytes: usize,
    mut init: F,
    len: usize,
) -> Option<RootRef<Array<T>>>
where
    F: FnMut() -> T,
    T: HasDescriptor,
{
    let data_bytes = size_of::<T>() * len;
    context::with_context_mut(move |x| {
        x.alloc_fast(
            TypeId::from(Array::<T>::DESCRIPTOR),
            data_bytes + extra_bytes,
        )
    })
    .or_else(move || {
        safepoint.before_safepoint();
        let ret = context::with_context_mut(move |x| {
            x.alloc_slow(
                TypeId::from(Array::<T>::DESCRIPTOR),
                data_bytes + extra_bytes,
            )
        });
        safepoint.after_safepoint();
        ret
    })
    .map(|x| {
        let mut ptr = x.get_ptr().data().cast::<MaybeUninit<Array<T>>>();
        // SAFETY: Allocator allocated correct sizing and stuffs, so its safe to write
        // we're writing header here
        unsafe { ptr.as_mut() }.write(Array {
            metadata: ArrayHeader { size: len },
            _phantom: PhantomData,
        });

        // SAFETY: We allocated with correct descriptor for given type
        // by constructing Descriptor, caller guarantee its correct. So
        // we trust it
        let mut ret = unsafe { RootRef::<Array<T>>::from_raw(x, ()) };

        // Now lets init the data
        Array::get_uninit_slice_mut(&mut ret)
            .iter_mut()
            .for_each(|x| {
                x.write(init());
            });

        ret
    })
}

pub fn alloc<'a, T, F>(
    safepoint: &mut dyn Safepoint,
    extra_bytes: usize,
    init: F,
) -> Option<RootRef<T>>
where
    F: FnOnce() -> T,
    T: HasDescriptor,
{
    context::with_context_mut(move |x| x.alloc_fast(TypeId::from(T::DESCRIPTOR), extra_bytes))
        .or_else(move || {
            safepoint.before_safepoint();
            let ret = context::with_context_mut(move |x| {
                x.alloc_slow(TypeId::from(T::DESCRIPTOR), extra_bytes)
            });
            safepoint.after_safepoint();
            ret
        })
        .map(|x| {
            let mut ptr = x.get_ptr().data().cast::<MaybeUninit<T>>();
            // SAFETY: Allocator allocated correct sizing and stuffs, so its safe to write
            unsafe { ptr.as_mut() }.write(init());

            // SAFETY: We allocated with correct descriptor for given type
            // by constructing Descriptor, caller guarantee its correct. So
            // we trust it
            unsafe { RootRef::from_raw(x, ()) }
        })
}
