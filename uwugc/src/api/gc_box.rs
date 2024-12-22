use crate::refs::GCRefRaw;

use super::{root_refs::{Exclusive, RootRef, Sendable, Shared, Unsendable}, Context, ConstructorScope, ObjectLikeTrait};

// Its logically behaves like Box<T> where it
// the parent structure owns it, therefore if
// parent mutable then GCBox<T> can be mutated
#[repr(transparent)]
pub struct GCBox<T: ObjectLikeTrait> {
  inner: GCRefRaw<T>
}

impl<T: ObjectLikeTrait> GCBox<T> {
  // NOTE: Sendable and Exclusive needed because by creating this GCBox
  // it is assumed to be accesible to other thread so the reference can't
  // be from somewhere which does not expect the reference to be shared to
  // other thread and Exclusive because caller thread needed to be only
  // owner of it
  pub fn new(reference: RootRef<Sendable, Exclusive, T>, _alloc_context: &mut ConstructorScope) -> Self {
    return Self {
      inner: GCRefRaw::new(RootRef::into_raw(reference).get_object_borrow())
    };
  }
  
  // NOTE: Unsendable needed because the reference loaded cannot
  // be sent to other thread because it might cause one thread might have mutable
  // and other shared (immutable) and references returned GCBox intended to be
  // only be used by caller thread
  pub fn load<'this, 'context: 'this>(&'this self, ctx: &'context Context) -> RootRef<'this, Unsendable, Shared, T> {
    let raw = self.inner.load(&ctx.inner, &mut ctx.inner.get_heap().gc.block_gc());
    // SAFETY: The data in there is owned by GCBox<T>
    // and Unsendable assures that references don't escape
    // away
    // SAFETY: GCBox<T> will never have null reference
    return unsafe { RootRef::new(raw.unwrap_unchecked()) };
  }
  
  // NOTE: Unsendable needed because the reference loaded cannot
  // be sent to other thread because it might cause one thread might have mutable
  // and other shared (immutable) and references returned GCBox intended to be
  // only be used by caller thread
  pub fn load_mut<'this, 'context: 'this>(&'this mut self, ctx: &'context Context) -> RootRef<'this, Unsendable, Exclusive, T> {
    let raw = self.inner.load(&ctx.inner, &mut ctx.inner.get_heap().gc.block_gc());
    // SAFETY: The data in there is owned by GCBox<T>
    // and Unsendable assures that references don't escape
    // away
    // SAFETY: GCBox<T> will never have null reference
    return unsafe { RootRef::new(raw.unwrap_unchecked()) };
  }
}

