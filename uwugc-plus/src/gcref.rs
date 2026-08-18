use std::{
    borrow::Cow, marker::PhantomData, mem::MaybeUninit, ptr::{self, NonNull}, sync::atomic::{AtomicPtr, Ordering}
};

use uwugc::ObjectPtr;

use crate::{Descriptor, HasDescriptor, RootRef, context};

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

pub struct GCBoxOption<T: Unpin + ?Sized + 'static> {
    inner: AtomicPtr<u8>,
    metadata: MaybeUninit<<T as std::ptr::Pointee>::Metadata>,
    _phantom: PhantomData<T>,
}

// # Safety
// We told where the pointer is, because we are the pointer
unsafe impl<T: Unpin> HasDescriptor for GCBoxOption<T> {
    const DESCRIPTOR: &'static crate::Descriptor = &unsafe { Descriptor::new(Cow::Borrowed(&[0]), size_of::<Self>()) };
}

// # Safety
// We told where the pointer is, because we are the pointer
unsafe impl<T: Unpin> HasDescriptor for GCBox<T> {
    const DESCRIPTOR: &'static crate::Descriptor = &unsafe { Descriptor::new(Cow::Borrowed(&[0]), size_of::<Self>()) };
}

impl<T: Unpin + ?Sized> GCBoxOption<T> {
    pub fn none() -> Self {
        Self {
            inner: AtomicPtr::new(ptr::null_mut()),
            metadata: MaybeUninit::uninit(),
            _phantom: PhantomData,
        }
    }

    // # Safety
    // By creating GCBoxOption you have to make sure its store
    // onto heap, where GC can find GCBox before reaching
    // any safepoints
    pub unsafe fn new(init: Option<RootRef<T>>) -> Self {
        let meta = init.as_ref()
            .map(RootRef::get_ptr)
            .map(|x| ptr::metadata(x));
        let ptr = init
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        Self {
            inner: AtomicPtr::new(ptr),
            metadata: meta.map(MaybeUninit::new).unwrap_or_else(MaybeUninit::uninit),
            _phantom: PhantomData,
        }
    }

    pub fn load(&self, ordering: Ordering) -> Option<RootRef<T>> {
        let ptr = NonNull::new(self.inner.load(ordering))?;
        // SAFETY: It is initialize if ptr is non null
        let metadata = unsafe { self.metadata.assume_init() };

        Some(context::with_context_mut(move |x| {
            // SAFETY: GCBoxOption will only contains valid GC pointer
            let root_ref = x.add_ptr(unsafe { ObjectPtr::from_nonnull(ptr) });

            // SAFETY: The object can be interpret as T, because no other ptr can be placed
            unsafe { RootRef::from_raw(root_ref, metadata) }
        }))
    }

    pub fn store(&mut self, ordering: Ordering, reference: Option<RootRef<T>>) {
        let meta = reference.as_ref()
            .map(RootRef::get_ptr)
            .map(|x| ptr::metadata(x));
        let ptr = reference
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        self.metadata = meta.map(MaybeUninit::new).unwrap_or_else(MaybeUninit::uninit);
        self.inner.store(ptr, ordering);
    }
}

pub struct GCBox<T: Unpin + ?Sized + 'static> {
    inner: GCBoxOption<T>,
}

impl<T: Unpin + ?Sized> GCBox<T> {
    // # Safety
    // By creating GCBox you have to make sure its store
    // onto heap, where GC can find GCBox before reaching
    // any safepoints
    pub unsafe fn new(init: RootRef<T>) -> Self {
        Self {
            inner: unsafe { GCBoxOption::new(Some(init)) },
        }
    }

    pub fn load(&self, ordering: Ordering) -> RootRef<T> {
        self.inner.load(ordering).expect("GCBox is nonnullable")
    }

    pub fn store(&mut self, ordering: Ordering, reference: RootRef<T>) {
        self.inner.store(ordering, Some(reference));
    }
}
