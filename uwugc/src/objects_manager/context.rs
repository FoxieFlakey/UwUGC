use std::{cell::UnsafeCell, marker::PhantomData, ptr, sync::{atomic::{self, Ordering}, Arc}, thread};

use portable_atomic::AtomicBool;

use crate::{descriptor::Describeable, gc::GCLockCookie, objects_manager::{Object, ObjectLikeTrait}};

use super::{AllocError, ObjectManager};

pub struct LocalObjectsChain {
  // Maintains start and end of chain
  start: UnsafeCell<Option<*const Object>>,
  end: UnsafeCell<Option<*const Object>>
}

// SAFETY: Accesses to this is protected by GC lock and lock on 'contexts'
// in ObjectManager structure
unsafe impl Sync for LocalObjectsChain {}
unsafe impl Send for LocalObjectsChain {}

impl LocalObjectsChain {
  pub fn new() -> Self {
    return Self {
      start: UnsafeCell::new(None),
      end: UnsafeCell::new(None)
    }
  }
  
  // Move all objects in local chain to global list
  // clearing local chain
  // SAFETY: Caller must ensure that 'owner' is actually the owner
  // and make sure that this chain isn't concurrently accessed
  // Concurrent access can be protected either by
  // 1. preventing Sweeper (the only other thing which concurrently access)
  //     from getting exclusive GC lock
  // 2. locks the 'contexts' as Sweeper also needs it
  pub unsafe fn flush_to_global(&self, owner: &ObjectManager) {
    // Make sure newest object added by mutator visible to current thread
    // (which might be other thread than the mutator)
    atomic::fence(Ordering::Acquire);
    
    // SAFETY: Caller make sure that LocalObjectsChain not concurrently accessed
    let (start, end) = unsafe { (&mut *self.start.get(), &mut *self.end.get()) };
    
    // Nothing to flush
    if start.is_none() {
      assert_eq!(start.is_none(), true);
      assert_eq!(end.is_none(), true);
      return;
    }
    
    // SAFETY: All objects in chain are valid, by design random object in middle
    // of chain cannot be deallocated safely
    unsafe {
      owner.add_chain_to_list(start.unwrap(), end.unwrap());
    }
    
    // Clear the list
    *start = None;
    *end = None;
    
    // Make the changes visible to the mutator so it can properly start new chain
    atomic::fence(Ordering::Release);
  }
}

pub struct ContextHandle<'a> {
  ctx: Arc<LocalObjectsChain>,
  owner: &'a ObjectManager,
  // Ensure that ContextHandle stays on same thread
  // by disallowing it to be Send or Sync
  _phantom: PhantomData<*const ()>
}

impl<'a> ContextHandle<'a> {
  pub(super) fn new(ctx: Arc<LocalObjectsChain>, owner: &'a ObjectManager) -> Self {
    return Self {
      owner,
      ctx,
      _phantom: PhantomData {}
    };
  }
  
  pub fn try_alloc<T: Describeable + ObjectLikeTrait>(&self, func: &mut dyn FnMut() -> T, _gc_lock_cookie: &mut GCLockCookie) -> Result<*mut Object, AllocError> {
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
      data: Box::new(func()),
      marked: AtomicBool::new(Object::compute_new_object_mark_bit(self.owner)),
      next: UnsafeCell::new(ptr::null_mut()),
      descriptor: T::get_descriptor(),
      total_size
    }));
    
    // Ensure changes made previously by potential flush_to_global
    // emptying the local list visible to this
    atomic::fence(Ordering::Acquire);
    
    // Add object to local chain
    // SAFETY: Safe because the concurrent access by other is protected by GC lock
    // See comment for LocalObjectsChain#flush_to_global method
    let start = unsafe { &mut *self.ctx.start.get() };
    let end = unsafe { &mut *self.ctx.end.get() };
    match start.as_mut() {
      // The list has some objects, append current 'start' to end of this object
      // SAFETY: The object isn't visible yet to other thread so its safe from
      // concurrent accesses
      Some(x) => unsafe { *obj.next.get() = *x },
      
      // The list was empty this object is the 'end' of current list
      None => *end = Some(obj)
    }
    
    // Update the 'start' so it point to newly made object
    *start = Some(obj);
    
    // Make sure potential flush_to_global can see latest items
    atomic::fence(Ordering::Release);
    return Ok(obj);
  }
}

impl Drop for ContextHandle<'_> {
  fn drop(&mut self) {
    let mut contexts = self.owner.contexts.lock();
    
    // Move all objects in current local chain to global list
    // SAFETY: Concurrent access can't happen because of Sweeper
    // needs 'contexts' to be locked
    unsafe { self.ctx.flush_to_global(self.owner); };
    
    // Remove context belonging to current thread
    contexts.remove(&thread::current().id());
  }
}

