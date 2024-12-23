use crate::heap::Heap as HeapInternal;
use super::{Context, HeapArc, Params};

impl Clone for HeapArc {
  fn clone(&self) -> Self {
    Self::from(self.inner.clone())
  }
}

impl HeapArc {
  #[must_use]
  pub fn new(params: Params) -> HeapArc {
    HeapArc::from(HeapInternal::new(params))
  }
  
  #[must_use]
  pub fn create_context(&self) -> Context {
    Context::from(HeapInternal::create_context(&self.inner))
  }
  
  #[must_use = "There is no side effect of this"]
  pub fn get_usage(&self) -> usize {
    self.inner.get_usage()
  }
}

