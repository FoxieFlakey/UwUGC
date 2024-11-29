use std::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

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
}

