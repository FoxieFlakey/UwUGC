use super::{Context, Describeable, Exclusive, ObjectConstructorContext, ObjectLikeTrait, RootRef, Sendable};

impl<'a> Context<'a> {
  pub fn alloc<T: Describeable + ObjectLikeTrait>(&mut self, initer: impl FnOnce(&mut ObjectConstructorContext) -> T) -> RootRef<'a, Sendable, Exclusive, T> {
    return self.inner.alloc(initer);
  }
}


