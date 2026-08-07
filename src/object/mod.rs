mod metadata;

pub use metadata::{
    Metadata as MetadataCompressed, MetadataEnum as ObjectKind, MetadataExpanded as Metadata,
};

// A pointer to object
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct ObjectPtr(*mut u8);

unsafe impl Send for ObjectPtr {}
unsafe impl Sync for ObjectPtr {}

impl ObjectPtr {
    // # Safety
    // Caller must make sure that pointer is valid object atleast sized MetadataCompressed
    // and contains valid metadata
    pub unsafe fn new(ptr: *mut u8) -> Self {
        Self(ptr)
    }

    #[expect(unused)]
    pub(crate) fn metadata(&self) -> Metadata {
        self.metadata_ref().get()
    }

    pub(crate) fn metadata_ref(&self) -> &MetadataCompressed {
        // SAFETY: We can make sure the pointer points to MetadataCompresed
        unsafe { self.0.cast::<MetadataCompressed>().as_ref_unchecked() }
    }

    #[expect(unused)]
    pub fn size(&self) -> usize {
        match self.metadata_ref().get().payload {
            ObjectKind::NotPlainOldData(_) => unimplemented!(),
            ObjectKind::PlainOldData(size) => size.value().try_into().unwrap(),
            ObjectKind::RefArray(len) => usize::try_from(len.value()).unwrap() * size_of::<u64>(),
        }
    }

    #[expect(unused)]
    pub fn data(&self) -> *mut u8 {
        // SAFETY: Object is only valid if its atleast size of MetadataCompressed, so
        // this never escapes "allocation"
        unsafe { self.0.byte_add(size_of::<MetadataCompressed>()) }
    }
}
