use std::{any::Any, ptr, sync::atomic::{AtomicBool, AtomicPtr, Ordering}};

use crate::objects_manager::{Object, ObjectRef};

use super::ObjectManager;

pub struct Context<'a> {
  pub(super) owner: &'a ObjectManager
}

// Ensure that Context stays on same thread
// by disallowing it to be Send or Sync
impl !Send for Context<'_> {}
impl !Sync for Context<'_> {}

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

