use super::{root_refs::{Exclusive, RootRef, Sendable}, Context, Describeable, ObjectConstructorContext, ObjectLikeTrait};

impl<'a> Context<'a> {
  pub fn alloc<T: Describeable + ObjectLikeTrait>(&mut self, initer: impl FnOnce(&mut ObjectConstructorContext) -> T) -> RootRef<'a, Sendable, Exclusive, T> {
    return self.inner.alloc(initer);
  }
  
  pub fn trigger_gc(&mut self) {
    self.inner.get_heap().run_gc();
  }
}


