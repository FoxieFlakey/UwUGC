mod metadata;

use std::ptr::NonNull;

pub use metadata::{
    Bit, Metadata as MetadataCompressed, MetadataEnum as ObjectKind, MetadataExpanded as Metadata,
};

// A pointer to object
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ObjectPtr(NonNull<u8>);

unsafe impl Send for ObjectPtr {}
unsafe impl Sync for ObjectPtr {}

impl ObjectPtr {
    // Returns None if ptr is null pointer
    // # Safety
    // Caller must make sure that pointer is valid object atleast sized MetadataCompressed
    // and contains valid metadata and came from GC context allocate function
    pub unsafe fn from_raw(ptr: *mut u8) -> Option<Self> {
        NonNull::new(ptr).map(ObjectPtr)
    }

    // # Safety
    // Caller must make sure that pointer is valid object atleast sized MetadataCompressed
    // and contains valid metadata and came from GC context allocate function
    pub unsafe fn from_nonnull(ptr: NonNull<u8>) -> Self {
        Self(ptr)
    }

    pub(crate) fn metadata(&self) -> Metadata {
        self.metadata_ref().get()
    }

    pub(crate) fn metadata_ref(&self) -> &MetadataCompressed {
        // SAFETY: We can make sure the pointer points to MetadataCompresed
        unsafe { self.0.cast::<MetadataCompressed>().as_ref() }
    }

    pub fn data(&self) -> NonNull<u8> {
        // SAFETY: Object is only valid if its atleast size of MetadataCompressed, so
        // this never escapes "allocation"
        unsafe { self.0.byte_add(size_of::<MetadataCompressed>()) }
    }

    pub fn into_raw(self) -> NonNull<u8> {
        self.0
    }
}
