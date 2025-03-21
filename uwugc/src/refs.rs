use std::{marker::PhantomData, ptr::{self, NonNull}, sync::atomic::Ordering};

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
  
  fn create_root_ref<'a, A: HeapAlloc>(ptr: *mut Object, ctx: &'a Context<A>, block_gc_cookie: &mut GCLockCookie<A>)  -> Option<RootRefRaw<'a, A, T>> {
    let root_ref = ctx.new_root_ref_from_ptr(NonNull::new(ptr)?, block_gc_cookie);
    let heap = ctx.get_heap();
    // SAFETY: Object is being referenced from root
    // therefore GC won't collect it and will remains valid
    heap.gc.load_barrier(unsafe { root_ref.get_object_ptr().as_ref() }, &heap.object_manager, block_gc_cookie);
    Some(root_ref)
  }
  
  pub fn swap<'a, A: HeapAlloc>(&self, ctx: &'a Context<A>, block_gc_cookie: &mut GCLockCookie<A>, root_ref: Option<&RootRefRaw<'a, A, T>>) -> Option<RootRefRaw<'a, A, T>>{
    // SAFETY: The data is always valid only need read_volatile to deter Rust from
    // optimizing that even with &mut because GC might see incorrect/illegal state
    let new_ptr = root_ref.map_or(ptr::null_mut(), |x| x.get_object_ptr().as_ptr());
    let old = unsafe { (*ptr::read_volatile(&&raw const self.ptr)).swap(new_ptr, Ordering::Relaxed) };
    Self::create_root_ref(old, ctx, block_gc_cookie)
  }
  
  pub fn store<'a, A: HeapAlloc>(&self, _ctx: &'a Context<A>, _block_gc_cookie: &mut GCLockCookie<A>, root_ref: Option<&RootRefRaw<'a, A, T>>) {
    // SAFETY: The data is always valid only need read_volatile to deter Rust from
    // optimizing that even with &mut because GC might see incorrect/illegal state
    let new_ptr = root_ref.map_or(ptr::null_mut(), |x| x.get_object_ptr().as_ptr());
    unsafe { (*ptr::read_volatile(&&raw const self.ptr)).store(new_ptr, Ordering::Relaxed) };
  }
  
  pub fn load<'a, A: HeapAlloc>(&self, ctx: &'a Context<A>, block_gc_cookie: &mut GCLockCookie<A>) -> Option<RootRefRaw<'a, A, T>> {
    // SAFETY: The data is always valid only need read_volatile to deter Rust from
    // optimizing that even with &mut because GC might see incorrect/illegal state
    let ptr = unsafe { (*ptr::read_volatile(&&raw const self.ptr)).load(Ordering::Relaxed) };
    Self::create_root_ref(ptr, ctx, block_gc_cookie)
  }
}

