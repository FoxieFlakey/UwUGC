use std::{any::Any, ptr, sync::{atomic::{AtomicBool, AtomicPtr, Ordering}, Arc}, thread};

use crate::{objects_manager::{Object, ObjectRef}, util::double_atomic_ptr::AtomicDoublePtr};

use super::ObjectManager;

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
    
    // SAFETY: All objects in chain are valid, by design random object in middle
    // of chain cannot be deallocated safely
    unsafe {
      owner.add_chain_to_list(start, end);
    }
  }
}

pub struct Context<'a> {
  ctx: Arc<LocalObjectsChain>,
  owner: &'a ObjectManager
}

// Ensure that ContextGuard stays on same thread
// by disallowing it to be Send or Sync
impl !Send for Context<'_> {}
impl !Sync for Context<'_> {}

impl<'a> Context<'a> {
  pub fn new(ctx: Arc<LocalObjectsChain>, owner: &'a ObjectManager) -> Self {
    return Self {
      owner,
      ctx
    };
  }
  
  pub fn alloc<T: Any + Sync + Send + 'static>(&self, func: impl FnOnce() -> T) -> ObjectRef<T> {
    let manager = self.owner;
    
    // Leak it and we'll handle it here
    let obj = Box::leak(Box::new(Object {
      data: Box::new(func()),
      marked: AtomicBool::new(false),
      next: AtomicPtr::new(ptr::null_mut()),
      total_size: 0
    }));
    
    let obj_ptr = obj as *mut Object as usize;
    println!("Allocated   : {obj_ptr:#016x}");
    
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
    
    obj.total_size = size_of_val(obj) + size_of_val(obj.data.as_ref());
    manager.used_size.fetch_add(obj.total_size, Ordering::Relaxed);
    return ObjectRef::new(obj);
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

impl Drop for Context<'_> {
  fn drop(&mut self) {
    // Move all objects in current local chain to global list
    self.flush_to_global();
    
    let mut contexts = self.owner.contexts.lock().unwrap();
    
    // Remove context belonging to current thread
    contexts.remove(&thread::current().id());
  }
}

