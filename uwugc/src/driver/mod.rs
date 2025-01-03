
pub mod stat_collector;

use crate::{allocator::HeapAlloc, heap::State as HeapState};

pub enum Action {
  RunGC,
  Pass
}

pub trait Driver<A: HeapAlloc>: Sync + Send + 'static {
  fn poll(&self, heap: &HeapState<A>) -> Action;
}



