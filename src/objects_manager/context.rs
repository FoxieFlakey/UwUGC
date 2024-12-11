use std::{ptr, sync::{atomic::{AtomicPtr, Ordering}, Arc}, thread};

use portable_atomic::AtomicBool;

use crate::{descriptor::Describeable, gc::GCLockCookie, objects_manager::{Object, ObjectLikeTrait}, util::double_atomic_ptr::AtomicDoublePtr};

use super::{AllocError, ObjectDataContainer, ObjectManager};

pub struct LocalObjectsChain {
  // Maintains start and end of chain
  chain: AtomicDoublePtr<Object, Object>
}

impl LocalObjectsChain {
  pub fn new() -> Self {
    return Self {
      chain: AtomicDoublePtr::new((ptr::null_mut(), ptr::null_mut()))
    }
  }
  
  // Move all objects in local chain to global list
  // clearing local chain
  // SAFETY: Caller must ensure that 'owner' is actually the owner
  pub unsafe fn flush_to_global(&self, owner: &ObjectManager) {
    // Relaxed ordering because doesnt need to access the object itself
    let (start, end) = self.chain.swap((ptr::null_mut(), ptr::null_mut()), Ordering::Acquire);
    
    // Nothing to flush
    if start == ptr::null_mut() {
      assert_eq!(start, ptr::null_mut());
      assert_eq!(end, ptr::null_mut());
      return;
    }
    
    // SAFETY: All objects in chain are valid, by design random object in middle
    // of chain cannot be deallocated safely
    unsafe {
      owner.add_chain_to_list(start, end);
    }
  }
}

pub struct ContextHandle<'a> {
  ctx: Arc<LocalObjectsChain>,
  owner: &'a ObjectManager
}

// Ensure that ContextHandle stays on same thread
// by disallowing it to be Send or Sync
impl !Send for ContextHandle<'_> {}
impl !Sync for ContextHandle<'_> {}

impl<'a> ContextHandle<'a> {
  pub(super) fn new(ctx: Arc<LocalObjectsChain>, owner: &'a ObjectManager) -> Self {
    return Self {
      owner,
      ctx
    };
  }
  
  pub fn try_alloc<T: Describeable + ObjectLikeTrait>(&self, func: &mut dyn FnMut() -> T, gc_lock_cookie: &mut GCLockCookie) -> Result<*mut Object, AllocError> {
    let manager = self.owner;
    let total_size = size_of::<Object>() + size_of::<T>();
    let mut current_usage = manager.used_size.load(Ordering::Relaxed);
    loop {
      let new_size = current_usage + total_size;
      if new_size >= manager.max_size {
        return Err(AllocError);
      }
      
      match manager.used_size.compare_exchange_weak(current_usage, new_size, Ordering::Relaxed, Ordering::Relaxed) {
        Ok(_) => break,
        Err(x) => current_usage = x
      }
    }
    
    // Leak it and we'll handle it here
    let obj = Box::leak(Box::new(Object {
      data: ObjectDataContainer::new(Box::new(func())),
      marked: AtomicBool::new(Object::compute_new_object_mark_bit(self.owner)),
      next: AtomicPtr::new(ptr::null_mut()),
      descriptor: T::get_descriptor(),
      total_size
    }));
    
    // Add object to local chain
    // NOTE: Relaxed because dont need to access data pointers returned by load
    let mut old = self.ctx.chain.load(Ordering::Relaxed);
    loop {
      // Ordering::Relaxed because not yet to make changes visible
      obj.next.store(old.0, Ordering::Relaxed);
      
      // NOTE: Ordering::Release on success because changes in the object should be visible
      // NOTE: Ordering::Relaxed on success because dont need data returned by load
      let new ;
      if old.0 == ptr::null_mut() {
        // Start of new chain, current object also is the last
        new = (obj as *mut Object, obj as *mut Object);
      } else {
        // Continuation of chain, current object is prepended
        new = (obj as *mut Object, old.1);
      }
      
      match self.ctx.chain.compare_exchange_weak(old, new, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => break,
        Err(actual) => old = actual
      }
    }
    
    drop(gc_lock_cookie);
    return Ok(obj);
  }
  
  // Move all objects in local chain to global list
  // clearing local chain
  pub fn flush_to_global(&self) {
    // SAFETY: This Context is created together with owner so it is correct owner
    unsafe {
      self.ctx.flush_to_global(self.owner);
    }
  }
}

impl Drop for ContextHandle<'_> {
  fn drop(&mut self) {
    // Move all objects in current local chain to global list
    self.flush_to_global();
    
    let mut contexts = self.owner.contexts.lock();
    
    // Remove context belonging to current thread
    contexts.remove(&thread::current().id());
  }
}

