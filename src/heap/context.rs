use std::{any::Any, cell::UnsafeCell, marker::PhantomData, pin::Pin, ptr, sync::{atomic, Arc}, thread};

use super::{Heap, RootEntry};
use crate::objects_manager::ContextHandle as ObjectManagerContextHandle;

pub struct ContextInner {
  head: Pin<Box<RootEntry>>
}

pub struct Context {
  inner: UnsafeCell<ContextInner>
}

impl Context {
  pub fn new() -> Self {
    let mut head = Box::pin(RootEntry {
      gc_state: ptr::null_mut(),
      obj: ptr::null_mut(),
      next: UnsafeCell::new(ptr::null_mut()),
      prev: UnsafeCell::new(ptr::null_mut())
    });
    
    *(head.next.get_mut()) = &mut *head;
    *(head.prev.get_mut()) = &mut *head;
    
    return Self {
      inner: UnsafeCell::new(ContextInner {
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
    let inner = &*self.inner.get();
    let head = inner.head.as_ref().get_ref();
    
    let mut current = &**head.next.get();
    // While 'current' is not the head as this linked list is circular
    while current as *const RootEntry != head as *const RootEntry {
      iterator(current);
      current = &**current.next.get();
    }
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
}

impl<T: Any + Send + Sync + 'static> Drop for RootRef<'_, T> {
  fn drop(&mut self) {
    // Corresponding RootEntry and RootRef are free'd together
    // therefore its safe after removing reference from root set
    let entry = unsafe { &mut *self.entry_ref };
    
    // Block GC as GC would see half modified root set if without
    // SAFETY: GCState is always valid
    let cookie = unsafe { &*entry.gc_state }.block();
    
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
    let gc_lock_cookie = self.owner.gc_state.block();
    let new_obj = self.obj_manager_ctx.alloc(initer);
    
    let entry = Box::pin(RootEntry {
      gc_state: &self.owner.gc_state,
      obj: new_obj,
      next: UnsafeCell::new(ptr::null_mut()),
      prev: UnsafeCell::new(ptr::null_mut())
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

