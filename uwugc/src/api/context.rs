use std::mem::MaybeUninit;

use crate::{allocator::GlobalHeap, heap::RootRefRaw};

use super::{ConstructorScope, Context, Describeable, ObjectLikeTrait, ReferenceType};

impl<'a> Context<'a> {
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub fn alloc<T: Describeable + ObjectLikeTrait>(&self, initer: impl FnOnce(&mut ConstructorScope) -> T) -> RootRefRaw<'a, GlobalHeap, T> {
    // SAFETY: Already properly initialize T
    unsafe { self.inner.alloc(|ctx, uninit| { uninit.write(initer(ctx)); }) }
  }
  
  /// # Safety
  ///
  /// Caller must make sure that initializer completely initalize T
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub unsafe fn alloc2<T: Describeable + ObjectLikeTrait>(&self, initer: impl FnOnce(&mut ConstructorScope, &mut MaybeUninit<T>)) -> RootRefRaw<'a, GlobalHeap, T> {
    // SAFETY: Caller already make sure to initialize T
    unsafe { self.inner.alloc(initer) }
  }
  
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub fn alloc_array<Ref: ReferenceType, const LEN: usize>(&self, initer: impl FnOnce(&mut ConstructorScope) -> [Ref; LEN]) -> RootRefRaw<'a, GlobalHeap, [Ref; LEN]> {
    // SAFETY: Already properly initialize T
    unsafe { self.inner.alloc_array(|ctx, uninit| { uninit.write(initer(ctx)); }) }
  }
  
  /// # Safety
  ///
  /// Initializer must properly initialize the array
  #[must_use = "If not used, this would create unnecessary GC pressure"]
  pub unsafe fn alloc_array2<Ref: ReferenceType, const LEN: usize>(&self, initer: impl FnOnce(&mut ConstructorScope, &mut MaybeUninit<[Ref; LEN]>)) -> RootRefRaw<'a, GlobalHeap, [Ref; LEN]> {
    // SAFETY: Caller already make sure to initialize the array
    unsafe { self.inner.alloc_array(initer) }
  }
  
  pub fn trigger_gc(&mut self) {
    self.inner.get_heap().run_gc(false);
  }
}


