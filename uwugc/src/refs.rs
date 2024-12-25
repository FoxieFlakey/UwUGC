use std::{marker::PhantomData, ptr, sync::atomic::Ordering};

use crate::allocator::HeapAlloc;
use portable_atomic::AtomicPtr;

use crate::{gc::GCLockCookie, heap::{Context, RootRefRaw}, objects_manager::Object, ObjectLikeTraitInternal};

#[repr(transparent)]
pub struct GCRefRaw<T: ObjectLikeTraitInternal> {
  ptr: AtomicPtr<Object>,
  _phantom: PhantomData<T>
}

impl<T: ObjectLikeTraitInternal> GCRefRaw<T> {
  pub fn new(data: *const Object) -> Self {
    Self {
      ptr: AtomicPtr::new(data.cast_mut()),
      _phantom: PhantomData {}
    }
  }
  
  #[expect(dead_code)]
  pub unsafe fn store<'a, A: HeapAlloc>(&self, _ctx: &'a Context<A>, root_ref: &RootRefRaw<'a, A, T>, _block_gc_cookie: &mut GCLockCookie<A>) {
    self.ptr.swap(ptr::from_ref(root_ref.get_object_borrow()).cast_mut(), Ordering::Relaxed);
  }
  
  pub fn load<'a, A: HeapAlloc>(&self, ctx: &'a Context<A>, block_gc_cookie: &mut GCLockCookie<A>) -> Option<RootRefRaw<'a, A, T>> {
    let ptr = self.ptr.load(Ordering::Relaxed);
    if ptr.is_null() {
      return None;
    }
    
    let root_ref = ctx.new_root_ref_from_ptr(ptr, block_gc_cookie);
    let heap = ctx.get_heap();
    heap.gc.load_barrier(root_ref.get_object_borrow(), &heap.object_manager, block_gc_cookie);
    Some(root_ref)
  }
}

