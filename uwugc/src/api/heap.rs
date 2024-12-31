use crate::{allocator::GlobalHeap, heap::Heap as HeapInternal};
use super::{Context, CycleState, GCStats, HeapArc, Params, HeapStats};

impl Clone for HeapArc {
  fn clone(&self) -> Self {
    Self::from(self.inner.clone())
  }
}

impl HeapArc {
  #[must_use]
  pub fn new(params: Params) -> HeapArc {
    HeapArc::from(HeapInternal::new(GlobalHeap {}, params))
  }
  
  #[must_use]
  pub fn create_context(&self) -> Context {
    Context::from(HeapInternal::create_context(&self.inner))
  }
  
  #[must_use = "There is no side effect of this"]
  pub fn get_usage(&self) -> usize {
    self.inner.get_usage()
  }
  
  #[must_use = "This does not have side effect"]
  pub fn get_cycle_state(&self) -> CycleState {
    self.inner.get_cycle_state()
  }
  
  #[must_use = "This does not have side effect"]
  pub fn get_gc_stats(&self) -> GCStats {
    self.inner.get_gc_stats()
  }
  
  #[must_use = "This does not have side effect"]
  pub fn get_lifetime_heap_stats(&self) -> HeapStats {
    self.inner.object_manager.get_lifetime_stats()
  }
}

