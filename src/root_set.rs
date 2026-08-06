use memmap2::MmapMut;

// # Guarantees
// the set and len, are always aligned to system page size
//
// # Safety
// implementer of this must follow contract defined in comments
// for clone and iterate. Its required for safety in GC marking
// process which assumes implementer of this folllows strict
// contract
pub unsafe trait RootSet: Sync + Send {
    // This must return the root set raw given at creation or clone
    fn get_raw(&self) -> &RootSetRaw;

    // RootSet must clone all data necessary for proper iteration
    // of GC pointers. Its current set is copied by GC to other
    // location at set and  old set must not be accessed
    //
    // Make this cheap is also highly desireable as this affects
    // STW time on SATB step
    fn clone_metadata(&self, set: RootSetRaw) -> Box<dyn RootSet>;

    // Visitor receives *mut u8 which is a GC pointer and returns
    // another *mut u8 to be replaced with.
    //
    // # Safety
    // Caller must make sure nothing uses the pointers inside RootSet. While
    // this function is running.
    unsafe fn map_pointers(&self, visitor: &mut dyn FnMut(*mut u8) -> *mut u8);
}

pub struct RootSetRaw {
    pub(crate) mapping: MmapMut,
    size: usize,
}

unsafe impl Send for RootSetRaw {}
unsafe impl Sync for RootSetRaw {}

impl RootSetRaw {
    pub(crate) fn new(size: usize) -> Self {
        Self {
            mapping: MmapMut::map_anon(size).unwrap(),
            size,
        }
    }

    #[expect(unused)]
    pub fn get_ptr(&self) -> *mut u8 {
        self.mapping.ptr_mut()
    }

    #[expect(unused)]
    pub fn get_size(&self) -> usize {
        self.size
    }

    // # Safety
    // there must be no active modification to the root set
    pub(crate) unsafe fn clone(&self) -> RootSetRaw {
        let mut cloned = Self::new(self.size);
        cloned.mapping.copy_from_slice(&self.mapping);
        cloned
    }
}

// Used when there no GC pointers
pub struct NoopRootSet {
    raw: RootSetRaw,
}

impl NoopRootSet {
    pub fn new(set: RootSetRaw) -> Self {
        Self { raw: set }
    }
}

unsafe impl RootSet for NoopRootSet {
    fn clone_metadata(&self, set: RootSetRaw) -> Box<dyn RootSet> {
        Box::new(Self::new(set))
    }

    fn get_raw(&self) -> &RootSetRaw {
        &self.raw
    }

    // there nothing in here so no-op
    unsafe fn map_pointers(&self, _: &mut dyn FnMut(*mut u8) -> *mut u8) {}
}
