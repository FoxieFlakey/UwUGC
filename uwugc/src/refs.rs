use std::{marker::PhantomData, ptr, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{gc::GCLockCookie, heap::{Context, RootRefRaw}, objects_manager::Object, ObjectLikeTrait};

#[repr(transparent)]
pub struct GCRefRaw<T: ObjectLikeTrait> {
  ptr: AtomicPtr<Object>,
  _phantom: PhantomData<T>
}

impl<T: ObjectLikeTrait> GCRefRaw<T> {
  pub fn new(data: *const Object) -> Self {
    Self {
      ptr: AtomicPtr::new(data.cast_mut()),
      _phantom: PhantomData {}
    }
  }
  
  #[expect(dead_code)]
  pub unsafe fn store<'a>(&self, _ctx: &'a Context, root_ref: &RootRefRaw<'a, T>, _block_gc_cookie: &mut GCLockCookie) {
    self.ptr.swap(ptr::from_ref(root_ref.get_object_borrow()).cast_mut(), Ordering::Relaxed);
  }
  
  pub fn load<'a>(&self, ctx: &'a Context, block_gc_cookie: &mut GCLockCookie) -> Option<RootRefRaw<'a, T>> {
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

