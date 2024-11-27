use std::{any::Any, ops::Deref, ptr, sync::atomic::{AtomicBool, AtomicPtr, Ordering}};

use crate::objects_manager::{Object, ObjectRef};

use super::ObjectManager;

pub struct Context<'a> {
  pub(super) owner: &'a ObjectManager
}

impl Context<'_> {
  pub fn alloc<T: Any + 'static>(&self, func: impl FnOnce() -> T) -> ObjectRef<T> {
    let manager = self.owner;
    
    // Leak it and we'll handle it here
    let obj = Box::leak(Box::new(Object {
      data: Box::new(func()),
      marked: AtomicBool::new(false),
      next: AtomicPtr::new(ptr::null_mut())
    }));
    
    let obj_ptr = obj as *mut Object as usize;
    println!("Allocated   : {obj_ptr:#016x}");
    
    // Try insert it to head
    // SAFETY: 'obj' is both start and end of 1 object length chain
    // and also just allocated it earlier
    unsafe {
      manager.add_chain_to_list(obj, obj);
    }
    
    let allocated_size = size_of_val(obj) + size_of_val(obj.data.as_ref());
    manager.used_size.fetch_add(allocated_size, Ordering::Relaxed);
    return ObjectRef::new(obj);
  }
}

pub struct ContextGuard<'a> {
  pub(super) ctx: Context<'a>
}

// Ensure that ContextGuard stays on same thread
// by disallowing it to be Send or Sync
impl !Send for ContextGuard<'_> {}
impl !Sync for ContextGuard<'_> {}

impl<'a> Deref for ContextGuard<'a> {
  type Target = Context<'a>;
  
  fn deref(&self) -> &Self::Target {
    return &self.ctx;
  }
}

