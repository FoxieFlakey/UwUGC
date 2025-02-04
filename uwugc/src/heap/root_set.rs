use std::{cell::UnsafeCell, marker::PhantomPinned, pin::Pin, ptr::NonNull};

use crate::{allocator::HeapAlloc, gc::GCState, objects_manager::Object};

pub struct RootEntry<A: HeapAlloc> {
  // The RootEntry itself cannot be *mut
  // but need to modify these fields because
  // there would be aliasing *mut (one from prev's next
  // and next's prev)
  next: UnsafeCell<NonNull<RootEntry<A>>>,
  prev: UnsafeCell<NonNull<RootEntry<A>>>,
  gc_state: NonNull<GCState<A>>,
  obj: NonNull<Object>,
  
  // RootEntry cannot be moved at will because
  // circular linked list need that guarantee
  _phantom: PhantomPinned
}

// SAFETY: It is only shared between GC thread and owning thread
// and GC thread, its being protected by GC locks
unsafe impl<A: HeapAlloc> Sync for RootEntry<A> {}
unsafe impl<A: HeapAlloc> Send for RootEntry<A> {}

impl<A: HeapAlloc> RootEntry<A> {
  pub fn get_gc_state(&self) -> NonNull<GCState<A>> {
    self.gc_state
  }
  
  pub fn get_obj_ptr(&self) -> NonNull<Object> {
    self.obj
  }
  
  // SAFETY: Caller make sure its safe to delete this root entry
  // and no possible concurrent access to the owning root set
  // and ensure that 'this' is valid
  pub unsafe fn delete(mut this: NonNull<RootEntry<A>>) {
    // SAFETY: Caller ensured that 'this' is valid pointer
    let this = unsafe { this.as_mut() };
    
    // SAFETY: Circular linked list is special that every next and prev
    // is valid so its safe and caller ensured that there are no concurrent
    // access
    let next_ref = unsafe { (*this.next.get()).as_ref() };
    let prev_ref = unsafe { (*this.prev.get()).as_ref() };
    
    // Actually removes
    // SAFETY: Caller ensures there are no concurrent access while
    // deletion is in progress
    unsafe {
      *next_ref.prev.get() = NonNull::from_ref(prev_ref);
      *prev_ref.next.get() = NonNull::from_ref(next_ref);
    };
    
    // Drop the root entry itself as its not in any root set
    // SAFETY: Nothing reference it anymore so it is safe
    // to be dropped and casted to *mut pointer
    let _ = unsafe { Box::from_raw(this) };
  }
  
  // Insert 'val' to next of this entry
  // Returns a *mut pointer to it and leaks it
  // SAFETY: Caller ensure that '&self' is the only borrow
  // TODO: Enforce it with '&mut self' instead
  unsafe fn insert(&self, val: Box<RootEntry<A>>) -> NonNull<RootEntry<A>> {
    // SAFETY: Internals of manually just setting pointers for linked list
    unsafe {
      let val = Box::leak(val);
      let val_nonnull = NonNull::from_mut(val);
      
      // Make 'val' prev points to this entry
      *val.prev.get() = NonNull::from_ref(self);
      
      // Make 'val' next points to entry next of this
      *val.next.get() = *self.next.get();
      
      // Make next entry's prev to point to 'val'
      // NOTE: 'next' is always valid in circular list
      *(*self.next.get()).as_ref().prev.get() = val_nonnull;
      
      // Make this entry's next to point to 'val'
      *self.next.get() = val_nonnull;
      NonNull::from_mut(val)
    }
  }
}

pub struct RootSet<A: HeapAlloc> {
  head: Pin<Box<RootEntry<A>>>
}

impl<A: HeapAlloc> RootSet<A> {
  // SAFETY: Caller must make sure that GCState<A> lives atleast
  // as long as the root set itself or RootEntry::get_gc_state
  // may return invalid pointer
  pub unsafe fn new(owning_gc: NonNull<GCState<A>>) -> Self {
    let head = Box::pin(RootEntry {
      gc_state: owning_gc,
      // Dangling next and prev because it will be overwritten later
      // and 'head' node don't have any object associated
      obj: NonNull::dangling(),
      next: UnsafeCell::new(NonNull::dangling()),
      prev: UnsafeCell::new(NonNull::dangling()),
      
      _phantom: PhantomPinned
    });
    
    // SAFETY: There no way concurrent access can happen yet
    // and root set need to be circular list
    unsafe {
      *head.next.get() = NonNull::from_ref(&*head);
      *head.prev.get() = NonNull::from_ref(&*head);
    }
    
    Self {
      head
    }
  }
  
  fn clear(&mut self) {
    let head = self.head.as_ref().get_ref();
    
    // SAFETY: Borrow checker ensured that nothing accessed the root set concurrently
    let mut current = unsafe { *head.next.get() };
    // While 'current' is not the head as this linked list is circular
    while current != NonNull::from_ref(head) {
      // SAFETY: Guaranteed by borrow checker that root set is not accessed concurrently
      let next = unsafe { *current.as_ref().next.get() };
      
      // Drop the root entry and remove it from set
      // SAFETY: Guaranteed by borrow checker that root set is not accessed concurrently
      let _ = unsafe { Box::from_raw(current.as_ptr()) };
      current = next;
    }
  }
  
  pub fn insert(&mut self, ptr: NonNull<Object>) -> NonNull<RootEntry<A>> {
    let entry = Box::new(RootEntry {
      obj: ptr,
      // Dangling next and prev because it will be overwritten later
      next: UnsafeCell::new(NonNull::dangling()),
      prev: UnsafeCell::new(NonNull::dangling()),
      gc_state: self.head.gc_state,
      
      _phantom: PhantomPinned
    });
    
    // SAFETY: self is &mut borrow so no other borrow
    // to 'head' can exist
    unsafe { self.head.insert(entry) }
  }
  
  pub fn take_snapshot(&self, buffer: &mut Vec<NonNull<Object>>) {
    self.for_each(|entry| {
      buffer.push(entry.obj);
    });
  }
  
  fn for_each(&self, mut iterator: impl FnMut(&RootEntry<A>)) {
    let head = self.head.as_ref().get_ref();
    
    // SAFETY: In circular buffer 'next' is always valid
    let mut current = unsafe { *head.next.get() };
    // While 'current' is not the head as this linked list is circular
    while current != NonNull::from_ref(head) {
      // SAFETY: Nodes pointer in circular linked list is valid
      // because immutable borrow which mean nodes can't be deallocated
      // or removed
      let current_ref = unsafe { current.as_ref() };
      
      iterator(current_ref);
      
      // SAFETY: In circular buffer 'next' is always valid
      current = unsafe { *current_ref.next.get() };
    }
  }
}

impl<A: HeapAlloc> Drop for RootSet<A> {
  fn drop(&mut self) {
    self.clear();
  }
}

