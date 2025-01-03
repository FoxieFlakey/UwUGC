
pub mod stat_collector;

use std::marker::PhantomData;

use stat_collector::StatItem;

use crate::{allocator::HeapAlloc, heap::State as HeapState};

pub enum Action {
  RunGC,
  Pass
}

pub trait Driver<A: HeapAlloc>: Send + 'static {
  fn poll(&mut self, heap: &HeapState<A>, stat: Option<&StatItem>) -> Action;
}

pub struct SimpleDriver<A: HeapAlloc, F: (FnMut(&HeapState<A>, Option<&StatItem>) -> Action) + Send + 'static> {
  func: F,
  _phantom: PhantomData<A>
}

impl<A: HeapAlloc + Sync, F: (FnMut(&HeapState<A>, Option<&StatItem>) -> Action) + Send + 'static> SimpleDriver<A, F> {
  pub fn new(func: F) -> Box<dyn Driver<A>> {
    Box::new(Self {
      _phantom: PhantomData {},
      func
    })
  }
}

impl<A: HeapAlloc, F: (FnMut(&HeapState<A>, Option<&StatItem>) -> Action) + Send + 'static> Driver<A> for SimpleDriver<A, F> {
  fn poll(&mut self, heap: &HeapState<A>, stat_item: Option<&StatItem>) -> Action {
    (self.func)(heap, stat_item)
  }
}

// Create default list of drivers to be used
pub fn drivers_list<A: HeapAlloc>() -> Vec<Box<dyn Driver<A>>> {
  Vec::from([
    SimpleDriver::new(|heap, _| {
      if heap.get_usage() >= heap.gc.get_params().trigger_size {
        return Action::RunGC;
      }
      
      Action::Pass
    })
  ])
}

