use std::{cell::UnsafeCell, collections::HashMap, pin::Pin, sync::{Arc, Mutex}, thread::{self, ThreadId}};

use context::Context;

use crate::{gc::GCState, objects_manager::{Object, ObjectManager}};

pub use context::ContextHandle;

mod context;

pub(super) struct RootEntry {
  next: UnsafeCell<*mut RootEntry>,
  prev: UnsafeCell<*mut RootEntry>,
  gc_state: *const GCState,
  obj: *mut Object
}

impl RootEntry {
  // Insert 'val' to next of this entry
  // Returns a *mut pointer to it and leaks it
  pub unsafe fn insert(&mut self, val: Pin<Box<RootEntry>>) -> *mut RootEntry {
    let val = Box::leak(Pin::into_inner(val));
    
    // Make 'val' prev points to this entry
    *val.prev.get_mut() = self;
    
    // Make 'val' next points to entry next of this
    *val.next.get_mut() = *self.next.get_mut();
    
    // Make next entry's prev to point to 'val'
    *(**self.next.get_mut()).prev.get_mut() = val;
    
    // Make this entry's next to point to 'val'
    *self.next.get_mut() = val;
    
    return val;
  }
}

pub struct Heap {
  object_manager: ObjectManager,
  contexts: Mutex<HashMap<ThreadId, Arc<Context>>>,
  
  gc_state: GCState
}

impl Heap {
  pub fn new() -> Self {
    return Self {
      object_manager: ObjectManager::new(),
      contexts: Mutex::new(HashMap::new()),
      gc_state: GCState::new()
    };
  }
  
  pub fn create_context(&self) -> ContextHandle {
    let mut contexts = self.contexts.lock().unwrap();
    let ctx = contexts.entry(thread::current().id())
      .or_insert_with(|| Arc::new(Context::new()));
    
    return ContextHandle::new(self, self.object_manager.create_context(), ctx.clone());
  }
  
  // SAFETY: Caller must ensure that mutators arent actively trying
  // to use the root concurrently
  pub unsafe fn take_root_snapshot_unlocked(&self, buffer: &mut Vec<*mut Object>) {
    let contexts = self.contexts.lock().unwrap();
    for ctx in contexts.values() {
      ctx.for_each_root(|entry| {
        // NOTE: Cast reference to *mut Object because after this
        // return caller must ensure that *mut Object is valid
        // because after this returns no lock ensures that GC isn't
        // actively collect that potential *mut Object
        buffer.push(entry.obj as *const Object as *mut Object);
      });
    }
  }
  
  pub fn take_root_snapshot(&self, buffer: &mut Vec<*mut Object>) {
    let cookie = self.gc_state.block_mutators();
    // SAFETY: Threads which modifies the root entries are blocked from
    // making changes
    unsafe {
      self.take_root_snapshot_unlocked(buffer);
    }
    drop(cookie);
  }
}


