use std::{any::Any, cell::SyncUnsafeCell, marker::PhantomData, pin::Pin, ptr, sync::{atomic, Arc}, thread};

use super::{Heap, RootEntry};
use crate::objects_manager::ContextHandle as ObjectManagerContextHandle;

pub struct ContextInner {
  head: Pin<Box<RootEntry>>
}

pub struct Context {
  inner: SyncUnsafeCell<ContextInner>
}

impl Context {
  pub fn new() -> Self {
    let mut head = Box::pin(RootEntry {
      gc_state: ptr::null_mut(),
      obj: ptr::null_mut(),
      next: SyncUnsafeCell::new(ptr::null_mut()),
      prev: SyncUnsafeCell::new(ptr::null_mut())
    });
    
    *(head.next.get_mut()) = &mut *head;
    *(head.prev.get_mut()) = &mut *head;
    
    return Self {
      inner: SyncUnsafeCell::new(ContextInner {
        head
      })
    };
  }
  
  // SAFETY: Caller ensures that thread managing the context does not
  // concurrently runs with this function (usually meant mutator
  // threads is being blocked)
  pub unsafe fn for_each_root(&self, mut iterator: impl FnMut(&RootEntry)) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    // SAFETY: Context is owned by the thread and only deallocated
    // by the same thread and no mutation
    let inner = unsafe { &*self.inner.get() };
    let head = inner.head.as_ref().get_ref();
    
    // SAFETY: In circular buffer 'next' is always valid
    let mut current = unsafe { &**head.next.get() };
    // While 'current' is not the head as this linked list is circular
    while current as *const RootEntry != head as *const RootEntry {
      iterator(current);
      // SAFETY: In circular buffer 'next' is always valid
      current = unsafe { &**current.next.get() };
    }
  }
  
  // SAFETY: Caller ensures that nothing can concurrently access the 
  // root set
  #[allow(unsafe_op_in_unsafe_fn)]
  pub unsafe fn clear_root_set(&self) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    
    let inner = &*self.inner.get();
    let head = inner.head.as_ref().get_ref();
    
    let mut current = *head.next.get();
    // While 'current' is not the head as this linked list is circular
    while current as *const RootEntry != head as *const RootEntry {
      let next = *(*current).next.get();
      
      let entry_ptr = current as usize;
      println!("Freed entry: {entry_ptr}");
      
      // Drop the root entry and remove it from set
      let _ = Box::from_raw(current);
      current = next;
    }
  }
}

impl Drop for Context {
  fn drop(&mut self) {
    // Current thread is last one with reference to this context
    // therefore its safe to clear it (to deallocate the root entries)
    unsafe { self.clear_root_set() };
  }
}

pub struct ContextHandle<'a> {
  ctx: Arc<Context>,
  obj_manager_ctx: ObjectManagerContextHandle<'a>,
  owner: &'a Heap
}

// ContextHandle will only stays at current thread
impl !Sync for ContextHandle<'_> {}
impl !Send for ContextHandle<'_> {}

pub struct RootRef<'a, T: Any + Send + Sync + 'static> {
  entry_ref: *mut RootEntry,
  phantom: PhantomData<&'a T>
}

// RootRef will only stays at current thread
impl<T> !Sync for RootRef<'_, T> {}
impl<T> !Send for RootRef<'_, T> {}

impl<'a, T: Any + Send + Sync + 'static> RootRef<'a, T> {
  pub fn borrow_inner(&self) -> &T {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    return unsafe { (*root_entry.obj).borrow_inner().unwrap() };
  }
  
  pub fn borrow_inner_mut(&mut self) -> &mut T {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    return unsafe { (*root_entry.obj).borrow_inner_mut().unwrap() };
  }
}

impl<T: Any + Send + Sync + 'static> Drop for RootRef<'_, T> {
  fn drop(&mut self) {
    // Corresponding RootEntry and RootRef are free'd together
    // therefore its safe after removing reference from root set
    let entry = unsafe { &mut *self.entry_ref };
    
    // Block GC as GC would see half modified root set if without
    // SAFETY: GCState is always valid
    let cookie = unsafe { &*entry.gc_state }.block_gc();
    
    // SAFETY: Circular linked list is special that every next and prev
    // is valid so its safe
    let next_ref = unsafe { &mut **entry.next.get_mut() };
    let prev_ref = unsafe { &mut **entry.prev.get_mut() };
    
    // Actually removes
    *next_ref.prev.get_mut() = prev_ref;
    *prev_ref.next.get_mut() = next_ref;
    
    // Let GC run again and Release fence to allow GC to see
    // removal of current entry (Acquire not needed as there
    // no other writer thread other than GC which only ever
    // does read)
    atomic::fence(atomic::Ordering::Release);
    drop(cookie);
    
    // Make sure that 'entry' reference will never be invalid
    // by telling Rust its lifetime ends here
    #[allow(dropping_references)]
    drop(entry);
    
    // Drop the "root_entry" itself as its unused now
    // SAFETY: Nothing reference it anymore so it is safe
    // to be dropped
    let _ = unsafe { Box::from_raw(self.entry_ref) };
  }
}

impl<'a> ContextHandle<'a> {
  pub(super) fn new(owner: &'a Heap, obj_manager_ctx: ObjectManagerContextHandle<'a>, ctx: Arc<Context>) -> Self {
    return Self {
      ctx,
      owner,
      obj_manager_ctx
    };
  }
  
  pub fn alloc<T: Any + Sync + Send + 'static>(&self, initer: impl FnOnce() -> T) -> RootRef<T> {
    let gc_lock_cookie = self.owner.gc_state.block_gc();
    let new_obj = self.obj_manager_ctx.alloc(initer);
    
    let entry = Box::new(RootEntry {
      gc_state: &self.owner.gc_state,
      obj: new_obj,
      next: SyncUnsafeCell::new(ptr::null_mut()),
      prev: SyncUnsafeCell::new(ptr::null_mut())
    });
    
    // SAFETY: Current thread is only owner of the head, and modification to it
    // is protected by GC locks
    let entry = unsafe { (*self.ctx.inner.get()).head.insert(entry) };
    
    // Allow GC to run again and Release fence to allow newly added value to be
    // visible to the GC
    atomic::fence(atomic::Ordering::Release);
    drop(gc_lock_cookie);
    return RootRef {
      entry_ref: entry,
      phantom: PhantomData {}
    };
  }
}

impl Drop for ContextHandle<'_> {
  fn drop(&mut self) {
    // Remove context belonging to current thread
    self.owner.contexts.lock().unwrap().remove(&thread::current().id());
  }
}

