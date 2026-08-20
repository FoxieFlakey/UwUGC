use std::{
    borrow::Cow, marker::PhantomData, pin::UnsafePinned, ptr::{self, NonNull}, sync::atomic::{AtomicPtr, Ordering}
};

use uwugc::ObjectPtr;

use crate::{Descriptor, HasDescriptor, RootRef};

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

pub struct GCBoxOption<T: Unpin + 'static> {
    inner: UnsafePinned<AtomicPtr<u8>>,
    _phantom: PhantomData<T>,
}

// I only need the opt out of typical aliasing rule.
impl<T: Unpin> Unpin for GCBoxOption<T> {}

// # Safety
// We told where the pointer is, because we are the pointer
unsafe impl<T: Unpin> HasDescriptor for GCBoxOption<T> {
    const DESCRIPTOR: &'static crate::Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[0]), size_of::<Self>()) };
}

// # Safety
// We told where the pointer is, because we are the pointer
unsafe impl<T: Unpin> HasDescriptor for GCBox<T> {
    const DESCRIPTOR: &'static crate::Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[0]), size_of::<Self>()) };
}

impl<T: Unpin> GCBoxOption<T> {
    pub fn none() -> Self {
        Self {
            inner: UnsafePinned::new(AtomicPtr::new(ptr::null_mut())),
            _phantom: PhantomData,
        }
    }

    // This pointer valid as long as no safepoint
    // occur (means the object is not moved)
    pub fn get_ptr(&self) -> Option<NonNull<T>> {
        // SAFETY: Both GC and mutator will only ever get shared
        // reference. The write/read are synchronized by atomics
        let loaded = unsafe {
                self.inner.get()
                    .cast_const()
                    .as_ref_unchecked()
                    .load(Ordering::Relaxed)
            };
        NonNull::new(loaded).map(|x| {
            // SAFETY: We only ever puts valid ObjectPtr so this is safe
            unsafe { ObjectPtr::from_nonnull(x) }.data().cast()
        })
    }

    // # Safety
    // By creating GCBoxOption you have to make sure its store
    // onto heap, where GC can find GCBox before reaching
    // any safepoints
    pub unsafe fn new(init: Option<RootRef<T>>) -> Self {
        let ptr = init
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        Self {
            inner: UnsafePinned::new(AtomicPtr::new(ptr)),
            _phantom: PhantomData,
        }
    }

    pub fn get_ref<'a>(&'a self) -> Option<&'a T> {
        // SAFETY: We have shared reference, this is safe
        self.get_ptr().map(|x| unsafe { x.as_ref() })
    }

    pub fn get_mut<'a>(&'a mut self) -> Option<&'a mut T> {
        // SAFETY: We have mutable reference, this is safe
        self.get_ptr().map(|mut x| unsafe { x.as_mut() })
    }

    pub fn store(&mut self, reference: Option<RootRef<T>>) {
        let ptr = reference
            .map(|x| {
                let raw = RootRef::into_raw(x);
                raw.get_ptr().into_raw().as_ptr()
            })
            .unwrap_or(ptr::null_mut());

        // SAFETY: Both GC and mutator will only ever get shared
        // reference. The write/read are synchronized by atomics
        unsafe {
            self.inner.get()
                .cast_const()
                .as_ref_unchecked()
                .store(ptr, Ordering::Relaxed);
        }
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
            inner: unsafe { GCBoxOption::new(Some(init)) },
        }
    }

    // This pointer valid as long as no safepoint
    // occur (means the object is not moved)
    pub fn get_ptr(&self) -> NonNull<T> {
        self.inner.get_ptr().unwrap()
    }

    pub fn get_ref<'a>(&'a self) -> &'a T {
        &self.inner.get_ref().unwrap()
    }

    pub fn get_mut<'a>(&'a mut self) -> &'a mut T {
        self.inner.get_mut().unwrap()
    }

    pub fn store(&mut self, reference: RootRef<T>) {
        self.inner.store(Some(reference));
    }
}
