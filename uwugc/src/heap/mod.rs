use std::{cell::UnsafeCell, collections::HashMap, marker::PhantomPinned, ops::Deref, sync::Arc, thread::{self, ThreadId}};
use parking_lot::Mutex;

use context::DataWrapper;
pub use context::{Context, RootRefRaw, ConstructorScope};

use crate::{gc::{GCParams, GCState}, objects_manager::{Object, ObjectManager}};

mod context;

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
pub struct Params {
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
  obj: *mut Object,
  
  // RootEntry cannot be moved at will because
  // circular linked list need that guarantee
  _phantom: PhantomPinned
}

// SAFETY: It is only shared between GC thread and owning thread
// and GC thread, its being protected by GC locks
unsafe impl Sync for RootEntry {}
unsafe impl Send for RootEntry {}

impl RootEntry {
  // Insert 'val' to next of this entry
  // Returns a *mut pointer to it and leaks it
  pub unsafe fn insert(&self, val: Box<RootEntry>) -> *mut RootEntry {
    // SAFETY: The caller must ensures that the root set is not concurrently accessed
    unsafe {
      let val = Box::leak(val);
      
      // Make 'val' prev points to this entry
      *val.prev.get() = self;
      
      // Make 'val' next points to entry next of this
      *val.next.get() = *self.next.get();
      
      // Make next entry's prev to point to 'val'
      // NOTE: 'next' is always valid in circular list
      *(**self.next.get()).prev.get() = val;
      
      // Make this entry's next to point to 'val'
      *self.next.get() = val;
      val
    }
  }
}

pub struct State {
  pub object_manager: ObjectManager,
  pub contexts: Mutex<HashMap<ThreadId, Arc<DataWrapper>>>,
  
  pub gc: GCState
}

pub struct Heap {
  __real_inner_arced_heap_state: Arc<State>
}

impl Deref for Heap {
  type Target = State;
  
  fn deref(&self) -> &Self::Target {
    &self.__real_inner_arced_heap_state
  }
}

impl Drop for Heap {
  fn drop(&mut self) {
    // Trigger GC shutdown and wait for it
    self.gc.shutdown_gc_and_wait();
  }
}

impl Heap {
  pub fn new(heap_params: Params) -> Arc<Self> {
    let this = Arc::new_cyclic(|weak_self| State {
      object_manager: ObjectManager::new(heap_params.max_size),
      contexts: Mutex::new(HashMap::new()),
      gc: GCState::new(heap_params.gc_params, weak_self.clone())
    });
    
    // Let GC run
    this.gc.unpause_gc();
    
    Arc::new(Heap {
      __real_inner_arced_heap_state: this
    })
  }
  
  #[must_use]
  pub fn create_context(&self) -> Context {
    let mut contexts = self.contexts.lock();
    let ctx = contexts.entry(thread::current().id())
      .or_insert_with(|| Arc::new(DataWrapper::new()));
    
    Context::new(self, self.object_manager.create_context(), ctx.clone())
  }
}

impl State {
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
    self.gc.run_gc();
  }
  
  pub fn get_usage(&self) -> usize {
    self.object_manager.get_usage()
  }
}


