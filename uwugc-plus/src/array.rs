use std::{
    marker::PhantomData,
    mem::MaybeUninit,
    ops::{Deref, DerefMut},
    ptr, slice,
};

use crate::{HasDescriptor, array_metadata::ArrayHeader};

#[repr(transparent)]
pub struct Array<T: Unpin + HasDescriptor + 'static> {
    pub(crate) metadata: ArrayHeader,
    pub(crate) _phantom: PhantomData<T>,
}

unsafe impl<T: Unpin + HasDescriptor + 'static> HasDescriptor for Array<T> {
    const DESCRIPTOR: &'static crate::Descriptor = &<T as HasDescriptor>::DESCRIPTOR.to_array();
}

impl<T: Unpin + HasDescriptor + 'static> Array<T> {
    pub(crate) fn get_uninit_slice_mut(&mut self) -> &mut [MaybeUninit<T>] {
        let ptr = ptr::from_mut(self).cast::<MaybeUninit<T>>();
        // SAFETY: This is sound, we're getting pointer right after Self
        // which is always 1byte after last byte that is valid
        let ptr = unsafe { ptr.byte_add(size_of::<Self>()) };

        // SAFETY: Trust that metadata correct
        unsafe { slice::from_raw_parts_mut(ptr, self.metadata.size) }
    }
}

impl<T: Unpin + HasDescriptor + 'static> Deref for Array<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        let ptr = ptr::from_ref(self).cast::<T>();
        // SAFETY: This is sound, we're getting pointer right after Self
        // which is always 1byte after last byte that is valid
        let ptr = unsafe { ptr.byte_add(size_of::<Self>()) };

        // SAFETY: Trust that metadata correct
        unsafe { slice::from_raw_parts(ptr, self.metadata.size) }
    }
}

impl<T: Unpin + HasDescriptor + 'static> DerefMut for Array<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let ptr = ptr::from_mut(self).cast::<T>();
        // SAFETY: This is sound, we're getting pointer right after Self
        // which is always 1byte after last byte that is valid
        let ptr = unsafe { ptr.byte_add(size_of::<Self>()) };

        // SAFETY: Trust that metadata correct
        unsafe { slice::from_raw_parts_mut(ptr, self.metadata.size) }
    }
}
