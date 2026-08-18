use std::{
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::context::RootRefRaw;

pub struct RootRef<T: Unpin> {
    raw: RootRefRaw,
    _phantom: PhantomData<T>,
}

impl<T: Unpin> RootRef<T> {
    // # Safety
    // Caller has to make sure that object being pointed is valid
    // to be interpreted as T
    pub(crate) unsafe fn from_raw(raw: RootRefRaw) -> Self {
        Self {
            raw,
            _phantom: PhantomData,
        }
    }

    pub fn store(this: &mut Self) {
        this.raw.store();
    }

    pub fn load(this: &mut Self) {
        this.raw.load();
    }
}

impl<T: Unpin> Deref for RootRef<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let ptr = self.raw.get_ptr().data().cast::<T>();

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        unsafe { ptr.as_ref() }
    }
}

impl<T: Unpin> DerefMut for RootRef<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let mut ptr = self.raw.get_ptr().data().cast::<T>();

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        unsafe { ptr.as_mut() }
    }
}
