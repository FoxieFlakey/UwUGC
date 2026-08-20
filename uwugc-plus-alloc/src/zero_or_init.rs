use std::{mem::MaybeUninit, ptr};

use uwugc_plus::HasDescriptor;

// This is special struct treated like MaybeUninit
// BUT its guaranteeed to be all zeros if uninit
// or init'ed data. This is because GC ignored pointer
// that is all zeros.
#[repr(transparent)]
pub struct ZeroOrInit<T: Unpin + HasDescriptor + 'static> {
    uninit: MaybeUninit<T>,
}

// SAFETY: This is structure is safe to be read by GC. Initialized
// mean pointers are valid or zero mean skip this pointer.
unsafe impl<T: Unpin + HasDescriptor + 'static> HasDescriptor for ZeroOrInit<T> {
    const DESCRIPTOR: &'static uwugc_plus::Descriptor = T::DESCRIPTOR;
}

impl<T: Unpin + HasDescriptor + 'static> ZeroOrInit<T> {
    pub fn zeroed() -> Self {
        Self {
            uninit: MaybeUninit::zeroed(),
        }
    }

    pub fn new(data: T) -> Self {
        Self {
            uninit: MaybeUninit::new(data),
        }
    }

    // Take the content out of current ZeroOrInit, making it
    // uninitialized
    pub fn take(&self) -> ZeroOrInit<T> {
        let mut result: MaybeUninit<T> = MaybeUninit::uninit();
        let src = self.uninit.as_ptr().cast::<u8>();
        let dest = result.as_mut_ptr().cast::<u8>();

        // SAFETY: We ensure both src and dest is valid. But
        // we don't care the initialization state as u8 valids
        // for all bit patterns
        unsafe { ptr::copy_nonoverlapping(src, dest, size_of::<T>()) };

        ZeroOrInit { uninit: result }
    }

    // # Safety
    // it is up to caller if its initialized
    pub unsafe fn assume_init_ref(&self) -> &T {
        // SAFETY: Caller ensured its initialized
        unsafe { self.uninit.assume_init_ref() }
    }

    // # Safety
    // it is up to caller if its initialized
    pub unsafe fn assume_init_mut(&mut self) -> &mut T {
        // SAFETY: Caller ensured its initialized
        unsafe { self.uninit.assume_init_mut() }
    }
}
