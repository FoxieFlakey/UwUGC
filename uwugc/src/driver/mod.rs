
pub mod stat_collector;

use crate::{allocator::HeapAlloc, heap::State as HeapState};

pub enum Action {
  RunGC,
  Pass
}

pub trait Driver<A: HeapAlloc>: Send + 'static {
  fn poll(&mut self, heap: &HeapState<A>) -> Action;
}



