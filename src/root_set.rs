use std::{any::Any, slice};

use crate::{mmap::Mmap, object::ObjectPtr};

// # Guarantees
// the set and len, are always aligned to system page size
//
// # Safety
// implementer of this must follow contract defined in comments
// for clone and iterate. Its required for safety in GC marking
// process which assumes implementer of this folllows strict
// contract
pub unsafe trait RootSet: Sync + Send + Any {
    // This must return the root set raw given at creation or clone
    fn get_raw(&self) -> &RootSetRaw;

    // RootSet must clone all data necessary for proper iteration
    // of GC pointers. Its current set is copied by GC to other
    // location at set and  old set must not be accessed
    //
    // Make this cheap is also highly desireable as this affects
    // STW time on SATB step
    fn clone_metadata(&self, set: RootSetRaw) -> Box<dyn RootSet>;

    /// Visitor receives *mut u8 which is a GC pointer and returns
    /// another *mut u8 to be replaced with. Implementer MUST allow
    /// portion of the region in RootSetRaw to be safely mutated by
    /// GC directly. But the changes are made by this function. It
    /// is guaranteed by GC. That mutator wont see half complete update
    /// to the raw set either via doing it in STW or via UFFD pagefaults
    /// to update on demand if GC hasn't catched up
    ///
    /// This is a little complicated so I'll show the code (not valid rust)
    ///
    /// ```rust,ignore
    /// // get root set from context. Assume its dumb
    /// // [*mut u8] array pointers
    ///
    /// // Thread Mutator                                   | Thread GC
    /// let set: &mut [*mut u8] = context.get_root_set();   |
    ///                                                     |
    /// // Do something like alloc object                   |
    /// let obj1: *mut u8 = context.alloc(283);             |
    /// do_something(obj2);                                 |
    ///                                                     |
    /// let obj2: *mut u8 = context.alloc(283);             |
    ///                                                     |
    /// set[0] = obj1;                                      |
    /// set[1] = obj2;                                      |
    /// // Assume GC started                                |
    /// context.safepoint();                                |
    /// // Paused                                           | // Start cycle
    ///                                                     | // ... normal GC cycle ... //
    ///                                                     | // ... skip to reference updates at root ... //
    ///                                                     | let backing_raw = memcpy_clone_of_raw_set(mutator_root_set.get_raw());
    ///                                                     | // Clone the root set, assuming GC already clone the raw set. The trait
    ///                                                     | // impl only need to clone metadata (a.k.a the data needed to properly
    ///                                                     | // figure where pointers are)
    ///                                                     | let snapshotted = mutator_root_set.clone_metadata(backing_raw);
    ///                                                     | snapshotted.map_pointers(|x| fixup_ptr(x));
    ///                                                     |
    ///                                                     | // Here what i mean by raw root set be allowed to be modified directly by GC
    ///                                                     | let current_raw = mutator_root_set.get_raw();
    ///                                                     | let updated_raw = snapshotted.get_raw();
    ///                                                     |
    ///                                                     | // Here what I meant. GC can copies from updated_raw to current_raw
    ///                                                     | // without consulting the root set trait. Only guarantee GC will give is
    ///                                                     | // mutator root set is not modified relative to one returned from clone_metadata
    ///                                                     | updated_raw.copy_from(current_raw);
    /// // Resumed                                          | // End cycle
    /// let set: &mut [*mut u8] = context.get_root_set();   |
    ///                                                     |
    /// // Reload pointer, which maybe moved                |
    /// let obj1 = set[0];                                  |
    /// let obj2 = set[1];                                  |
    ///                                                     |
    /// do_something_else(obj2);                            |
    /// ```
    ///
    /// Mechanism on GC like before, allows GC to defer copying and map_pointers to be concurrent via userfaultfd or other mechanisms
    /// without needing cooperation from root set implementation
    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr);

    // This like map_pointers, but less stricter read only operations. This is
    // implemented in term of map_pointers. Implementer should override this
    // if there faster way for read only.
    fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr));
}

pub struct RootSetRaw {
    pub(crate) mapping: Mmap,
    size: usize,
}

unsafe impl Send for RootSetRaw {}
unsafe impl Sync for RootSetRaw {}

impl RootSetRaw {
    pub(crate) fn new(size: usize) -> Self {
        Self {
            size,
            mapping: Mmap::map(size, true, true, true).unwrap(),
        }
    }

    pub fn get_ptr(&self) -> *mut u8 {
        self.mapping.get_ptr()
    }

    pub fn get_size(&self) -> usize {
        self.size
    }

    // # Safety
    // there must be no active modification to the root set
    pub(crate) unsafe fn clone(&self) -> RootSetRaw {
        let cloned = Self::new(self.size);

        // SAFETY: We just made cloned, and nobody access so &mut is safe
        let dest = unsafe { slice::from_raw_parts_mut(cloned.get_ptr(), cloned.get_size()) };

        // SAFETY: Caller ensures there no active modification or &mut so this is safe
        let src = unsafe { slice::from_raw_parts(self.get_ptr(), self.get_size()) };

        dest.copy_from_slice(src);
        cloned
    }
}

// Used when there no GC pointers
#[expect(unused)]
pub struct NoopRootSet {
    raw: RootSetRaw,
}

impl NoopRootSet {
    #[expect(unused)]
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
    fn map_pointers(&mut self, _: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {}
    fn iter_pointers(&self, _: &mut dyn FnMut(&ObjectPtr)) {}
}
