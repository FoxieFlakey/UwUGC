use std::sync::{RwLock, RwLockReadGuard};

pub struct GCState {
  gc_lock: RwLock<()>
}

pub struct GCLockCookie<'a> {
  _cookie: RwLockReadGuard<'a, ()>
}

impl GCState {
  pub fn new() -> GCState {
    return GCState {
      gc_lock: RwLock::new(())
    }
  }
  
  pub fn block(&self) -> GCLockCookie {
    return GCLockCookie {
      _cookie: self.gc_lock.read().unwrap()
    }
  }
}

