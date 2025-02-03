use std::{cell::UnsafeCell, marker::PhantomPinned};

use crate::{allocator::HeapAlloc, gc::GCState, objects_manager::Object};

pub struct RootEntry<A: HeapAlloc> {
  // The RootEntry itself cannot be *mut
  // but need to modify these fields because
  // there would be aliasing *mut (one from prev's next
  // and next's prev)
  pub(super) next: UnsafeCell<*const RootEntry<A>>,
  pub(super) prev: UnsafeCell<*const RootEntry<A>>,
  pub(super) gc_state: *const GCState<A>,
  pub(super) obj: *mut Object,
  
  // RootEntry cannot be moved at will because
  // circular linked list need that guarantee
  pub(super) _phantom: PhantomPinned
}

// SAFETY: It is only shared between GC thread and owning thread
// and GC thread, its being protected by GC locks
unsafe impl<A: HeapAlloc> Sync for RootEntry<A> {}
unsafe impl<A: HeapAlloc> Send for RootEntry<A> {}

impl<A: HeapAlloc> RootEntry<A> {
  // Insert 'val' to next of this entry
  // Returns a *mut pointer to it and leaks it
  pub unsafe fn insert(&self, val: Box<RootEntry<A>>) -> *mut RootEntry<A> {
    // SAFETY: The caller must ensures that the root set is not concurrently accessed
    unsafe {
      let val = Box::leak(val);
      
      // Make 'val' prev points to this entry
      *val.prev.get() = self;
      
      // Make 'val' next points to entry next of this
      *val.next.get() = *self.next.get();
      
      // Make next entry's prev to point to 'val'
      // NOTE: 'next' is always valid in circular list
      *(**self.next.get()).prev.get() = val;
      
      // Make this entry's next to point to 'val'
      *self.next.get() = val;
      val
    }
  }
}

