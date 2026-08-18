use std::{
    marker::PhantomData,
    ptr::{self, NonNull},
    sync::atomic::{AtomicPtr, Ordering},
};

use uwugc::ObjectPtr;

use crate::{RootRef, context};

// This type is what you'll put inside objects for GC pointers
// you must not overwrite it once its in heap. Implementing
// Descriptor and point to this where you might overwrite like
//
// struct Object {
//   field: GCBox<u32>,
// }
//
// // This is disallowed! necessary atomics will be gone and
// // breaks GC during marking phase. This is strange yes but
// // in safe code this can't happen as you cannot impl HasDescriptor
// // without acknowledging this IS BANNED and uwugc-plus refuses to
// // allocate objects without HasDescriptor
// object.field = GCBox::new(...)

#[repr(transparent)]
pub struct GCBoxOption<T: Unpin + 'static> {
    inner: AtomicPtr<u8>,
    _phantom: PhantomData<T>,
}

impl<T: Unpin> GCBoxOption<T> {
    // # Safety
    // By creating GCBoxOption you have to make sure its store
    // onto heap, where GC can find GCBox before reaching
    // any safepoints
    pub fn new(init: Option<RootRef<T>>) -> Self {
        let ptr = init
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        Self {
            inner: AtomicPtr::new(ptr),
            _phantom: PhantomData,
        }
    }

    pub fn load(&self, ordering: Ordering) -> Option<RootRef<T>> {
        let ptr = NonNull::new(self.inner.load(ordering))?;
        Some(context::with_context_mut(move |x| {
            // SAFETY: GCBoxOption will only contains valid GC pointer
            let root_ref = x.add_ptr(unsafe { ObjectPtr::from_nonnull(ptr) });

            // SAFETY: The object can be interpret as T, because no other ptr can be placed
            unsafe { RootRef::from_raw(root_ref) }
        }))
    }

    pub fn store(&mut self, ordering: Ordering, reference: Option<RootRef<T>>) {
        let ptr = reference
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        self.inner.store(ptr, ordering);
    }
}

pub struct GCBox<T: Unpin + 'static> {
    inner: GCBoxOption<T>,
}

impl<T: Unpin> GCBox<T> {
    // # Safety
    // By creating GCBox you have to make sure its store
    // onto heap, where GC can find GCBox before reaching
    // any safepoints
    pub unsafe fn new(init: RootRef<T>) -> Self {
        Self {
            inner: GCBoxOption::new(Some(init)),
        }
    }

    pub fn load(&self, ordering: Ordering) -> RootRef<T> {
        self.inner.load(ordering).expect("GCBox is nonnullable")
    }

    pub fn store(&mut self, ordering: Ordering, reference: RootRef<T>) {
        self.inner.store(ordering, Some(reference));
    }
}
