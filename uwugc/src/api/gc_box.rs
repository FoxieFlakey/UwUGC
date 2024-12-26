use std::ptr;

use crate::{allocator::GlobalHeap, refs::GCRefRaw};

use super::{root_refs::{Exclusive, RootRef, Sendable, Shared, Unsendable}, Context, ConstructorScope, ObjectLikeTrait};

// Its logically behaves like Box<T> where it
// the parent structure owns it, therefore if
// parent mutable then GCBox<T> can be mutated
// and it is nullable (there this may own nothing)
#[repr(transparent)]
pub struct GCNullableBox<T: ObjectLikeTrait> {
  inner: GCRefRaw<T>
}

impl<T: ObjectLikeTrait> GCNullableBox<T> {
  // NOTE: Sendable and Exclusive needed because by creating this GCBox
  // it is assumed to be accesible to other thread so the reference can't
  // be from somewhere which does not expect the reference to be shared to
  // other thread and Exclusive because caller thread needed to be only
  // owner of it
  pub fn new(reference: Option<RootRef<Sendable, Exclusive, GlobalHeap, T>>, _alloc_context: &mut ConstructorScope) -> Self {
    let ptr = reference.map_or(ptr::null(), |reference| {
      RootRef::into_raw(reference).get_object_ptr().as_ptr()
    });
    
    Self {
      inner: GCRefRaw::new(ptr)
    }
  }
  
  // NOTE: Unsendable needed because the reference loaded cannot
  // be sent to other thread because it might cause one thread might have mutable
  // and other shared (immutable) and references returned GCBox intended to be
  // only be used by caller thread
  pub fn load<'this, 'context: 'this>(&'this self, ctx: &'context Context) -> Option<RootRef<'this, Unsendable, Shared, GlobalHeap, T>> {
    self.inner.load(&ctx.inner, &mut ctx.inner.get_heap().gc.block_gc())
      .map(|raw| {
        // SAFETY: The data in there is owned by GCBox<T>
        // and Unsendable assures that references don't escape
        // away
        unsafe { RootRef::new(raw) }
      })
  }
  
  // NOTE: Unsendable needed because the reference loaded cannot
  // be sent to other thread because it might cause one thread might have mutable
  // and other shared (immutable) and references returned GCBox intended to be
  // only be used by caller thread
  pub fn load_mut<'this, 'context: 'this>(&'this mut self, ctx: &'context Context) -> Option<RootRef<'this, Unsendable, Exclusive, GlobalHeap, T>> {
    self.inner.load(&ctx.inner, &mut ctx.inner.get_heap().gc.block_gc())
      .map(|raw| {
        // SAFETY: The data in there is owned by GCBox<T>
        // and Unsendable assures that references don't escape
        // away
        unsafe { RootRef::new(raw) }
      })
  }
  
  // Swap this box's content with another, returns old one (which is now
  // free to use by caller as there no longer potential problem with sending
  // it to other thread)
  pub fn swap<'context>(&mut self, ctx: &'context Context, other: Option<RootRef<'context, Sendable, Exclusive, GlobalHeap, T>>) -> Option<RootRef<'context, Sendable, Exclusive, GlobalHeap, T>> {
    let raw_ref = other.map(|x| RootRef::into_raw(x));
    self.inner.swap(&ctx.inner, &mut ctx.inner.get_heap().gc.block_gc(), raw_ref.as_ref())
      .map(|raw| {
        // SAFETY: The box no longer contain the reference
        // and its transferred to the caller and &mut ensure
        // there no other reference to this box
        unsafe { RootRef::new(raw) }
      })
  }
}

// A non-null counterpart of GCNullableBox
#[repr(transparent)]
pub struct GCBox<T: ObjectLikeTrait> {
  inner: GCNullableBox<T>
}

impl<T: ObjectLikeTrait> GCBox<T> {
  pub fn new(reference: RootRef<Sendable, Exclusive, GlobalHeap, T>, alloc_context: &mut ConstructorScope) -> Self {
    Self {
      inner: GCNullableBox::new(Some(reference), alloc_context)
    }
  }
  
  pub fn load<'this, 'context: 'this>(&'this self, ctx: &'context Context) -> RootRef<'this, Unsendable, Shared, GlobalHeap, T> {
    // SAFETY: GCBox<T> will never be null/nothing
    unsafe { self.inner.load(ctx).unwrap_unchecked() }
  }
  
  pub fn load_mut<'this, 'context: 'this>(&'this mut self, ctx: &'context Context) -> RootRef<'this, Unsendable, Exclusive, GlobalHeap, T> {
    // SAFETY: GCBox<T> will never be null/nothing
    unsafe { self.inner.load_mut(ctx).unwrap_unchecked() }
  }
  
  pub fn swap<'context>(&mut self, ctx: &'context Context, other: RootRef<'context, Sendable, Exclusive, GlobalHeap, T>) -> RootRef<'context, Sendable, Exclusive, GlobalHeap, T> {
    // SAFETY: GCBox<T> will never be null/nothing
    unsafe { self.inner.swap(ctx, Some(other)).unwrap_unchecked() }
  }
}

