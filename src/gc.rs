use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::heap::Heap;

pub struct GCState {
  gc_lock: RwLock<()>
}

pub struct GCLockCookie<'a> {
  _cookie: RwLockReadGuard<'a, ()>
}

pub struct GCExclusiveLockCookie<'a> {
  _cookie: RwLockWriteGuard<'a, ()>
}

impl GCState {
  pub fn new() -> GCState {
    return GCState {
      gc_lock: RwLock::new(())
    }
  }
  
  pub fn block_gc(&self) -> GCLockCookie {
    return GCLockCookie {
      _cookie: self.gc_lock.read().unwrap()
    }
  }
  
  pub fn block_mutators(&self) -> GCExclusiveLockCookie {
    return GCExclusiveLockCookie {
      _cookie: self.gc_lock.write().unwrap()
    }
  }
  
  pub fn run_gc(&self, heap: &Heap) {
    // Step 1 (STW): Take root snapshot and take objects in heap snapshot
    let block_mutator_cookie = self.block_mutators();
    
    let mut root_snapshot = Vec::new();
    // SAFETY: Just blocked the mutators
    unsafe { heap.take_root_snapshot_unlocked(&mut root_snapshot) };
    let sweeper = heap.object_manager.create_sweeper();
    
    drop(block_mutator_cookie);
    
    // Step 2 (Concurrent): Mark objects
    for obj in root_snapshot {
      // SAFETY: Object is reference from root that mean
      // mutator still using it therefore GC must keep it alive
      let obj = unsafe { &*obj };
      
      // Mark it
      obj.mark();
    }
    
    // Step 3 (Concurrent): Sweep dead objects and reset mark flags 
    // SAFETY: just marked live objects and dead objects
    // is well dead
    unsafe { sweeper.sweep_and_reset_mark_flag() };
  }
}

