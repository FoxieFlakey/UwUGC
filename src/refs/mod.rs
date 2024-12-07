// Those are in object GCRef basically AtomicPtr in disguise

use std::{any::Any, marker::PhantomData, ptr, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{heap::{ContextHandle, RootRefMut}, objects_manager::Object};

// Nonnullable mutable reference to an object
// basically acts like &mut T
#[repr(transparent)]
pub struct GCRef<T: Any + Send + Sync + 'static> {
  inner: GCNullableMutRef<T>
}

impl<T: Any + Send + Sync + 'static> GCRef<T> {
  pub fn new<'a>(content: &mut RootRefMut<T>) -> Self {
    return Self {
      inner: GCNullableMutRef::new(Some(content))
    }
  }
  
  pub fn load<'a>(&self, ctx: &'a ContextHandle) -> RootRefMut<'a, T> {
    // SAFETY: This type enforces nonnull content
    return unsafe { self.inner.load(ctx).unwrap_unchecked() };
  }
  
  pub fn store<'a>(&mut self, ctx: &'a ContextHandle, content: &mut RootRefMut<T>) {
    return self.inner.store(ctx, Some(content));
  }
}

// Nullable mutable reference to object
// basically acts like Option<&mut T>
#[repr(transparent)]
pub struct GCNullableMutRef<T: Any + Send + Sync + 'static> {
  ptr: AtomicPtr<Object>,
  phantom: PhantomData<T>
}

impl<T: Any + Send + Sync + 'static> GCNullableMutRef<T> {
  fn new_impl<'a>(content: *const Object) -> Self {
    return Self {
      ptr: AtomicPtr::new(content as *mut Object),
      phantom: PhantomData {}
    }
  }
  
  fn turn_optional_root_ref_mut_to_nullable_object_pointer(option: &Option<&mut RootRefMut<T>>) -> *const Object {
    return option.as_ref()
      .map(|x| x.get_object_borrow() as *const Object)
      .unwrap_or(ptr::null());
  }
  
  pub fn new<'a>(content: Option<&mut RootRefMut<T>>) -> Self {
    return Self::new_impl(Self::turn_optional_root_ref_mut_to_nullable_object_pointer(&content));
  }
  
  pub fn load<'a>(&self, ctx: &'a ContextHandle) -> Option<RootRefMut<'a, T>> {
    let gc_state = &ctx.get_heap().gc_state;
    let blocked_gc_cookie = gc_state.block_gc();
    let ptr = self.ptr.load(Ordering::Relaxed);
    if ptr.is_null() {
      return None;
    }
    
    // SAFETY: Just blocked the GC from running and also
    // if this called from mutator, they must have a strong
    // reference to root, so it can't be GC'ed before GC is
    // blocked
    let root_ref = unsafe { ctx.new_root_ref_from_ptr::<T>(ptr) };
    
    // Also call conditional load barrier because this reference is reachable
    gc_state.load_barrier(root_ref.get_object_borrow(), &ctx.get_heap().object_manager);
    drop(blocked_gc_cookie);
    return Some(root_ref);
  }
  
  pub fn store<'a>(&mut self, _ctx: &'a ContextHandle, content: Option<&mut RootRefMut<T>>) {
    self.ptr.store(Self::turn_optional_root_ref_mut_to_nullable_object_pointer(&content) as *mut Object, Ordering::Relaxed);
  }
}


