use std::{cell::UnsafeCell, marker::PhantomData, pin::Pin, ptr, sync::{atomic, Arc}, thread};

use super::{Heap, RootEntry};
use crate::{descriptor::Describeable, gc::GCLockCookie, objects_manager::{context::ContextHandle as ObjectManagerContextHandle, Object, ObjectLikeTrait}, root_refs::{Exclusive, RootRef, Sendable}};

pub struct ContextInner {
  head: Pin<Box<RootEntry>>
}

pub struct Context {
  inner: UnsafeCell<ContextInner>
}

// SAFETY: Manually enforces safety of concurrently accessing it
// by GC lock, and GC is only the other thread which reads this
// while the owning thread is the only writer
unsafe impl Sync for Context {}

// This type exists so that any API can enforce that
// it is being constructed/called inside a special context which
// only be made available by the 'alloc' function as creating
// GC refs anywhere else is always unsafe due the assumptions
// it needs
pub struct ObjectConstructorContext {
  _private: ()
}

impl Context {
  pub fn new() -> Self {
    let mut head = Box::pin(RootEntry {
      gc_state: ptr::null_mut(),
      obj: ptr::null_mut(),
      next: ptr::null_mut(),
      prev: ptr::null_mut()
    });
    
    head.next = &mut *head;
    head.prev = &mut *head;
    
    return Self {
      inner: UnsafeCell::new(ContextInner {
        head
      })
    };
  }
  
  // SAFETY: Caller ensures that thread managing the context does not
  // concurrently runs with this function (usually meant mutator
  // threads is being blocked)
  pub(super) unsafe fn for_each_root(&self, mut iterator: impl FnMut(&RootEntry)) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    // SAFETY: Caller ensured mutators are blocked so nothing modifies this
    let inner = unsafe { &*self.inner.get() };
    let head = inner.head.as_ref().get_ref();
    
    // SAFETY: In circular buffer 'next' is always valid
    let mut current = unsafe { &*head.next };
    // While 'current' is not the head as this linked list is circular
    while current as *const RootEntry != head as *const RootEntry {
      iterator(current);
      // SAFETY: In circular buffer 'next' is always valid
      current = unsafe { &*current.next };
    }
  }
  
  // SAFETY: Caller ensures that nothing can concurrently access the 
  // root set
  #[allow(unsafe_op_in_unsafe_fn)]
  pub unsafe fn clear_root_set(&self) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    
    // SAFETY: Caller ensured mutators are blocked so nothing modifies this
    let inner = unsafe { &*self.inner.get() };
    let head = inner.head.as_ref().get_ref();
    
    let mut current = head.next;
    // While 'current' is not the head as this linked list is circular
    while current as *const RootEntry != head as *const RootEntry {
      let next = (*current).next;
      
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
  owner: &'a Heap,
  // ContextHandle will only stays at current thread
  _phantom: PhantomData<*const ()>
}

pub struct RootRefRaw<'a, T: ObjectLikeTrait> {
  entry_ref: *mut RootEntry,
  _phantom: PhantomData<&'a T>,
  // RootRef will only stays at current thread
  _force_not_send_sync: PhantomData<*const ()>
}

impl<'a, T: ObjectLikeTrait> RootRefRaw<'a, T> {
  // SAFETY: The root reference may not be safe in face of
  // data race, it is up to caller to ensure its safe
  pub unsafe fn borrow_inner(&self) -> &T {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    
    // SAFETY: Type already statically checked by Rust
    // via this type's T
    return unsafe { (*root_entry.obj).borrow_inner::<T>() };
  }
  
  // SAFETY: The root reference may not be safe in face of
  // data race, it is up to caller to ensure its safe
  pub unsafe fn borrow_inner_mut(&mut self) -> &mut T {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    
    // SAFETY: Type already statically checked by Rust
    // via this type's T
    return unsafe { (*root_entry.obj).borrow_inner_mut() };
  }
  
  pub fn get_object_borrow(&self) -> &Object {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    // SAFETY: Objects accessible by this root reference guaranteed
    // to be alive by the GC
    return unsafe { &*root_entry.obj };
  }
}

impl<T: ObjectLikeTrait> Drop for RootRefRaw<'_, T> {
  fn drop(&mut self) {
    // Corresponding RootEntry and RootRef are free'd together
    // therefore its safe after removing reference from root set
    let entry = unsafe { &mut *self.entry_ref };
    
    // Block GC as GC would see half modified root set if without
    // SAFETY: GCState is always valid
    let cookie = unsafe { &*entry.gc_state }.block_gc();
    
    // SAFETY: Circular linked list is special that every next and prev
    // is valid so its safe
    let next_ref = unsafe { &mut *entry.next };
    let prev_ref = unsafe { &mut *entry.prev };
    
    // Actually removes
    next_ref.prev = prev_ref;
    prev_ref.next = next_ref;
    
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
      obj_manager_ctx,
      _phantom: PhantomData {}
    };
  }
  
  pub fn get_heap(&self) -> &Heap {
    return &self.owner;
  }
  
  pub fn new_root_ref_from_ptr<T: ObjectLikeTrait>(&self, ptr: *mut Object, _gc_lock_cookie: &mut GCLockCookie) -> RootRefRaw<'a, T> {
    let entry = Box::new(RootEntry {
      gc_state: &self.owner.gc_state,
      obj: ptr,
      next: ptr::null_mut(),
      prev: ptr::null_mut()
    });
    
    // SAFETY: Current thread is only owner of the head, and modification to it
    // is protected by GC locks by requirement of '_gc_lock_cookie' mutable reference
    // which requires that GC is blocked to have a reference to it
    //
    // therefore, current thread modifies it and GC won't be able to concurrently
    // access it
    let entry = unsafe { (*self.ctx.inner.get()).head.insert(entry) };
    
    // Release fence to allow newly added value to be
    // visible to the GC
    atomic::fence(atomic::Ordering::Release);
    return RootRefRaw {
      entry_ref: entry,
      _phantom: PhantomData {},
      _force_not_send_sync: PhantomData {}
    };
  }
  
  pub fn alloc<T: Describeable + ObjectLikeTrait>(&mut self, initer: impl FnOnce(&mut ObjectConstructorContext) -> T) -> RootRef<'a, Sendable, Exclusive, T> {
    // Shouldn't panic if try_alloc succeded once, and with this
    // method this function shouldnt try alloc again
    let mut special_ctx = ObjectConstructorContext { _private: () };
    let mut inited = Some(initer);
    let mut must_init_once = || inited.take().unwrap()(&mut special_ctx);
    
    let mut gc_lock_cookie = self.owner.gc_state.block_gc();
    let mut obj = self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie);
    
    if obj.is_err() {
      drop(gc_lock_cookie);
      println!("Out of memory, triggering GC!");
      self.owner.run_gc();
      gc_lock_cookie = self.owner.gc_state.block_gc();
      
      obj = self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie);
      if obj.is_err() {
        panic!("Heap run out of memory!");
      }
    }
    
    let root_ref = self.new_root_ref_from_ptr(obj.unwrap(), &mut gc_lock_cookie);
    // SAFETY: The object reference is exclusively owned by this thread
    return unsafe { RootRef::new(root_ref) };
  }
}

impl Drop for ContextHandle<'_> {
  fn drop(&mut self) {
    // Remove context belonging to current thread
    self.owner.contexts.lock().remove(&thread::current().id());
  }
}

