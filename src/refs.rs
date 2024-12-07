use std::{any::Any, marker::PhantomData, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{heap::{ContextHandle, RootRefMut}, objects_manager::Object};

// An in object GCRef basically AtomicPtr in disguise
// and it is nonnullable
#[repr(transparent)]
pub struct GCRef<T: Any + Send + Sync + 'static> {
  ptr: AtomicPtr<Object>,
  phantom: PhantomData<T>
}

impl<T: Any + Send + Sync + 'static> GCRef<T> {
  pub fn new<'a>(content: &mut RootRefMut<T>) -> GCRef<T> {
    return Self {
      ptr: AtomicPtr::new(content.get_object_borrow() as *const Object as *mut Object),
      phantom: PhantomData {}
    }
  }
  
  pub fn load<'a>(&self, ctx: &'a ContextHandle) -> RootRefMut<'a, T> {
    let gc_state = &ctx.get_heap().gc_state;
    let blocked_gc_cookie = gc_state.block_gc();
    // SAFETY: Just blocked the GC from running and also
    // if this called from mutator, they must have a strong
    // reference to root, so it can't be GC'ed before GC is
    // blocked
    let root_ref = unsafe { ctx.new_root_ref_from_ptr::<T>(self.ptr.load(Ordering::Relaxed)) };
    
    // Also call conditional load barrier because this reference is reachable
    gc_state.load_barrier(root_ref.get_object_borrow(), &ctx.get_heap().object_manager);
    drop(blocked_gc_cookie);
    return root_ref;
  }
  
  pub fn store<'a>(&mut self, _ctx: &'a ContextHandle, content: &mut RootRefMut<T>) {
    self.ptr.store(content.get_object_borrow() as *const Object as *mut Object, Ordering::Relaxed);
  }
}


