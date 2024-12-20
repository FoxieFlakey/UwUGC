use std::{marker::PhantomData, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{gc::GCLockCookie, heap::context::{ContextHandle, RootRefRaw}, objects_manager::{Object, ObjectLikeTrait}};

#[repr(transparent)]
pub struct GCRefRaw<T: ObjectLikeTrait> {
  ptr: AtomicPtr<Object>,
  _phantom: PhantomData<T>
}

impl<T: ObjectLikeTrait> GCRefRaw<T> {
  pub fn new(data: *const Object) -> Self {
    return Self {
      ptr: AtomicPtr::new(data as *mut Object),
      _phantom: PhantomData {}
    }
  }
  
  #[expect(dead_code)]
  pub unsafe fn store<'a>(&self, _ctx: &'a ContextHandle, root_ref: &RootRefRaw<'a, T>, _block_gc_cookie: &mut GCLockCookie) {
    self.ptr.swap(root_ref.get_object_borrow() as *const Object as *mut Object, Ordering::Relaxed);
  }
  
  pub fn load<'a>(&self, ctx: &'a ContextHandle, block_gc_cookie: &mut GCLockCookie) -> Option<RootRefRaw<'a, T>> {
    let ptr = self.ptr.load(Ordering::Relaxed);
    return Some(ctx.new_root_ref_from_ptr(ptr, block_gc_cookie));
  }
}

