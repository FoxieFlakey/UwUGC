use std::{
    marker::{PhantomData, Unsize},
    ops::{CoerceUnsized, Deref, DerefMut},
};

use crate::context::RootRefRaw;

pub struct RootRef<T: Unpin + ?Sized> {
    raw: RootRefRaw,

    // TODO: Something better than wasting a pointer??
    // on Sized case. its duplicate of raw's pointer
    // its only exists for CoerceUnsize works
    only_care_for_metadata: *mut T,
    _phantom: PhantomData<T>,
}

impl<T: Unpin> RootRef<T> {
    // # Safety
    // Caller has to make sure that object being pointed is valid
    // to be interpreted as T
    pub(crate) unsafe fn from_raw(raw: RootRefRaw) -> Self {
        Self {
            only_care_for_metadata: raw.get_ptr().into_raw().as_ptr().cast::<T>(),
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
        match reference.only_care_for_metadata.as_mut_array() {
            Some(ptr) => Ok(RootRef {
                only_care_for_metadata: ptr,
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
        let ptr = self.raw.get_ptr().data().cast::<T>();
        let ptr = ptr.as_ptr().with_metadata_of(self.only_care_for_metadata);

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        // And ptr cannot be null because it came from NonNull
        unsafe { ptr.as_ref_unchecked() }
    }
}

impl<T: Unpin> DerefMut for RootRef<[T]> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let ptr = self.raw.get_ptr().data().cast::<T>();
        let ptr = ptr.as_ptr().with_metadata_of(self.only_care_for_metadata);

        // SAFETY: existence of RootRefRaw implies that current thread
        // is in 'context' implying GC is forbidden to move pointers
        // so pointer is valid
        // And ptr cannot be null because it came from NonNull
        unsafe { ptr.as_mut_unchecked() }
    }
}

// Allow RootRef<[u8; 512]> be coerced to RootRef<[u8]>
// and others
impl<T, U> CoerceUnsized<RootRef<U>> for RootRef<T>
where
    T: ?Sized + Unsize<U> + Unpin,
    U: ?Sized + Unpin,
{
}
