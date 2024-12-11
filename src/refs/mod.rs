use std::{marker::PhantomData, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{gc::GCLockCookie, heap::{ContextHandle, RootRefRaw}, objects_manager::{Object, ObjectLikeTrait}};

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
  
  pub unsafe fn store<'a>(&self, _ctx: &'a ContextHandle, root_ref: &RootRefRaw<'a, T>, _block_gc_cookie: &mut GCLockCookie) {
    self.ptr.swap(root_ref.get_object_borrow() as *const Object as *mut Object, Ordering::Relaxed);
  }
  
  pub unsafe fn load<'a>(&self, ctx: &'a ContextHandle, _block_gc_cookie: &mut GCLockCookie) -> Option<RootRefRaw<'a, T>>{
    let ptr = self.ptr.load(Ordering::Relaxed);
    // SAFETY: Have blocked the GC UwU due existence of mutable borrow of GCLockCookie
    return Some(unsafe { ctx.new_root_ref_from_ptr(ptr) });
  }
}

