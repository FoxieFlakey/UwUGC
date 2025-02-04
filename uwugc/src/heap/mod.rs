use std::{collections::HashMap, ops::Deref, ptr::NonNull, sync::Arc, thread::{self, ThreadId}};
use crate::{allocator::HeapAlloc, gc::{CycleState, GCStats}};
use parking_lot::Mutex;

use context::ContextData;
pub use context::{Context, RootRefRaw, ConstructorScope};

use crate::{gc::{GCParams, GCState}, objects_manager::{Object, ObjectManager}};

mod context;
mod root_set;
pub(super) use root_set::RootEntry;

// NOTE: This is considered public API
// therefore be careful with breaking changes
#[derive(Clone)]
pub struct Params {
  pub gc_params: GCParams,
  pub max_size: usize
}

pub struct State<A: HeapAlloc> {
  pub object_manager: ObjectManager<A>,
  pub contexts: Mutex<HashMap<ThreadId, Arc<ContextData<A>>>>,
  
  pub gc: GCState<A>
}

pub struct Heap<A: HeapAlloc> {
  __real_inner_arced_heap_state: Arc<State<A>>
}

impl<A: HeapAlloc> Deref for Heap<A> {
  type Target = State<A>;
  
  fn deref(&self) -> &Self::Target {
    &self.__real_inner_arced_heap_state
  }
}

impl<A: HeapAlloc> Drop for Heap<A> {
  fn drop(&mut self) {
    // Trigger GC shutdown and wait for it
    self.gc.shutdown_gc_and_wait();
  }
}

impl<A: HeapAlloc> Heap<A> {
  pub fn new(allocator: A, heap_params: Params) -> Arc<Self> {
    let this = Arc::new_cyclic(|weak_self| State {
      object_manager: ObjectManager::new(allocator, heap_params.max_size),
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
  pub fn create_context(&self) -> Context<A> {
    let mut contexts = self.contexts.lock();
    let ctx = contexts.entry(thread::current().id())
      .or_insert_with(|| {
        Arc::new(
          // SAFETY: GC lives longer than context data so it is safe
          unsafe { ContextData::new(NonNull::from_ref(&self.gc)) }
        )
      });
    
    Context::new(self, self.object_manager.create_context(), ctx.clone())
  }
}

impl<A: HeapAlloc> State<A> {
  // SAFETY: Caller must ensure that mutators arent actively trying
  // to use the root concurrently
  pub unsafe fn take_root_snapshot_unlocked(&self, buffer: &mut Vec<NonNull<Object>>) {
    let contexts = self.contexts.lock();
    for ctx in contexts.values() {
      // SAFETY: Its caller responsibility to make sure there are no
      // concurrent modification to the root set
      unsafe {
        ctx.take_root_set_snapshot(buffer);
      }
    }
  }
  
  pub fn run_gc(&self, is_for_oom: bool) {
    self.gc.run_gc(is_for_oom);
  }
  
  pub fn get_usage(&self) -> usize {
    self.object_manager.get_usage()
  }
  
  pub fn get_cycle_state(&self) -> CycleState {
    self.gc.get_cycle_state()
  }
  
  pub fn get_gc_stats(&self) -> GCStats {
    self.gc.get_gc_stats()
  }
}


