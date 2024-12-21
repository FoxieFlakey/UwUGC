use std::{cell::UnsafeCell, collections::HashMap, sync::Arc, thread::{self, ThreadId}};
use parking_lot::Mutex;

use context::{Context, ContextHandle};

use crate::{gc::{GCParams, GCState}, objects_manager::{Object, ObjectManager}};

pub mod context;

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
pub struct HeapParams {
  pub gc_params: GCParams,
  pub max_size: usize
}

pub(super) struct RootEntry {
  // The RootEntry itself cannot be *mut
  // but need to modify these fields because
  // there would be aliasing *mut (one from prev's next
  // and next's prev)
  next: UnsafeCell<*const RootEntry>,
  prev: UnsafeCell<*const RootEntry>,
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
  // SAFETY: The caller must ensures that the root set is not concurrently accessed
  #[allow(unsafe_op_in_unsafe_fn)]
  pub unsafe fn insert(&mut self, val: Box<RootEntry>) -> *mut RootEntry {
    let val = Box::leak(val);
    
    // Make 'val' prev points to this entry
    *val.prev.get() = self;
    
    // Make 'val' next points to entry next of this
    // SAFETY: The caller ensures that the root set is not concurrently accessed
    *val.next.get() = *self.next.get();
    
    // Make next entry's prev to point to 'val'
    // NOTE: 'next' is always valid in circular list
    *(**self.next.get()).prev.get() = val;
    
    // Make this entry's next to point to 'val'
    *self.next.get() = val;
    
    return val;
  }
}

pub struct Heap {
  pub object_manager: ObjectManager,
  pub contexts: Mutex<HashMap<ThreadId, Arc<Context>>>,
  
  pub gc_state: GCState
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
  pub unsafe fn take_root_snapshot_unlocked(&self, buffer: &mut Vec<*const Object>) {
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
          buffer.push(entry.obj);
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


