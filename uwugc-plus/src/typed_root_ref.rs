use std::{
    marker::{PhantomData, Unsize},
    ops::{Deref, DerefMut}, ptr,
};

use crate::context::RootRefRaw;

pub struct RootRef<T: Unpin + ?Sized> {
    raw: RootRefRaw,
    metadata: <T as std::ptr::Pointee>::Metadata,
    _phantom: PhantomData<T>,
}

impl<T: Unpin> RootRef<T> {
    // # Safety
    // Caller has to make sure that object being pointed is valid
    // to be interpreted as T
    pub(crate) unsafe fn from_raw(raw: RootRefRaw) -> Self {
        Self {
            metadata: ptr::metadata(raw.get_ptr().into_raw().as_ptr().cast::<T>()),
            raw,
            _phantom: PhantomData,
        }
    }
}

impl<T: Unpin + ?Sized> RootRef<T> {
    pub fn into_raw(self) -> RootRefRaw {
        self.raw
    }

    pub fn store(this: &mut Self) {
        this.raw.store();
    }

    pub fn load(this: &mut Self) {
        this.raw.load();
    }

    pub fn coerce<U>(this: Self) -> RootRef<U>
        where
            T: Unsize<U>,
            U: ?Sized + Unpin,
    {
        RootRef {
            metadata: ptr::metadata(Self::get_ptr(&this) as *mut U),
            raw: this.raw,
            _phantom: PhantomData
        }
    }

    fn get_ptr(this: &Self) -> *mut T {
        let ptr = this.raw.get_ptr().data().as_ptr();
        ptr::from_raw_parts_mut(ptr, this.metadata)
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

impl<T: Unpin> RootRef<[T]> {
    pub fn as_array<const N: usize>(reference: Self) -> Result<RootRef<[T; N]>, Self> {
        match Self::get_ptr(&reference).as_mut_array() {
            Some(ptr) => Ok(RootRef {
                metadata: ptr::metadata::<[T; N]>(ptr),
                raw: reference.raw,
                _phantom: PhantomData,
            }),
            None => Err(reference),
        }
    }
}

impl<T: Unpin> Deref for RootRef<[T]> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        let ptr = Self::get_ptr(self);

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        // And ptr cannot be null because it came from NonNull
        unsafe { ptr.as_ref_unchecked() }
    }
}

impl<T: Unpin> DerefMut for RootRef<[T]> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let ptr = Self::get_ptr(self);

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        // And ptr cannot be null because it came from NonNull
        unsafe { ptr.as_mut_unchecked() }
    }
}
