use std::{cell::SyncUnsafeCell, collections::HashMap, sync::Arc, thread::{self, ThreadId}};
use parking_lot::Mutex;

use context::Context;

use crate::{gc::{GCParams, GCState}, objects_manager::{Object, ObjectManager}};

pub use context::ContextHandle;
pub use context::RootRef;

mod context;

pub struct HeapParams {
  pub gc_params: GCParams,
  pub max_size: usize
}

pub(super) struct RootEntry {
  next: SyncUnsafeCell<*mut RootEntry>,
  prev: SyncUnsafeCell<*mut RootEntry>,
  gc_state: *const GCState,
  obj: *mut Object
}

// SAFETY: It is only shared between GC thread and owning thread
// and GC thread, its being protected by GC locks
unsafe impl Sync for RootEntry {}
unsafe impl Send for RootEntry {}

impl RootEntry {
  // Insert 'val' to next of this entry
  // Returns a *mut pointer to it and leaks it
  pub unsafe fn insert(&mut self, val: Box<RootEntry>) -> *mut RootEntry {
    let val = Box::leak(val);
    
    // Make 'val' prev points to this entry
    *val.prev.get_mut() = self;
    
    // Make 'val' next points to entry next of this
    *val.next.get_mut() = *self.next.get_mut();
    
    // Make next entry's prev to point to 'val'
    // SAFETY: 'next' is always valid in circular list
    unsafe { *(**self.next.get_mut()).prev.get_mut() = val };
    
    // Make this entry's next to point to 'val'
    *self.next.get_mut() = val;
    
    return val;
  }
}

pub struct Heap {
  pub(crate) object_manager: ObjectManager,
  pub(crate) contexts: Mutex<HashMap<ThreadId, Arc<Context>>>,
  
  pub(crate) gc_state: GCState
}

impl Heap {
  pub fn new(heap_params: HeapParams) -> Arc<Self> {
    let this = Arc::new_cyclic(|weak_self| Self {
      object_manager: ObjectManager::new(heap_params.max_size),
      contexts: Mutex::new(HashMap::new()),
      gc_state: GCState::new(heap_params.gc_params, weak_self.clone())
    });
    
    // Let GC run
    this.gc_state.unpause_gc();
    return this;
  }
  
  pub fn create_context(&self) -> ContextHandle {
    let mut contexts = self.contexts.lock();
    let ctx = contexts.entry(thread::current().id())
      .or_insert_with(|| Arc::new(Context::new()));
    
    return ContextHandle::new(self, self.object_manager.create_context(), ctx.clone());
  }
  
  // SAFETY: Caller must ensure that mutators arent actively trying
  // to use the root concurrently
  pub unsafe fn take_root_snapshot_unlocked(&self, buffer: &mut Vec<*mut Object>) {
    let contexts = self.contexts.lock();
    for ctx in contexts.values() {
      // SAFETY: Its caller responsibility to make sure there are no
      // concurrent modification to the root set
      unsafe {
        ctx.for_each_root(|entry| {
          // NOTE: Cast reference to *mut Object because after this
          // return caller must ensure that *mut Object is valid
          // because after this returns no lock ensures that GC isn't
          // actively collect that potential *mut Object
          buffer.push(entry.obj as *const Object as *mut Object);
        });
      }
    }
  }
  
  pub fn run_gc(&self) {
    self.gc_state.run_gc();
  }
  
  pub fn get_usage(&self) -> usize {
    return self.object_manager.get_usage();
  }
}


