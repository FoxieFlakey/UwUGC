use crate::allocator::GlobalHeap;

use super::{root_refs::{Exclusive, RootRef, Sendable}, ConstructorScope, Context, Describeable, ObjectLikeTrait, ReferenceType};

impl<'a> Context<'a> {
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub fn alloc<T: Describeable + ObjectLikeTrait>(&self, initer: impl FnOnce(&mut ConstructorScope) -> T) -> RootRef<'a, Sendable, Exclusive, GlobalHeap, T> {
    self.inner.alloc(initer)
  }
  
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub fn alloc_array<Ref: ReferenceType, const LEN: usize>(&self, initer: impl FnOnce(&mut ConstructorScope) -> [Ref; LEN]) -> RootRef<'a, Sendable, Exclusive, GlobalHeap, [Ref; LEN]> {
    self.inner.alloc_array(initer)
  }
  
  pub fn trigger_gc(&mut self) {
    self.inner.get_heap().run_gc();
  }
}


