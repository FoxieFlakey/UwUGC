use std::{any::Any, marker::PhantomData, sync::{Arc, Mutex}, thread};

use intrusive_collections::LinkedList;
use intrusive_collections::LinkedListLink;

use super::{Heap, RootEntry, RootEntryAdapter};
use crate::objects_manager::ContextHandle as ObjectManagerContextHandle;

struct ContextInner {
  root_set: LinkedList<RootEntryAdapter>,
  root_set_size: usize
}

pub struct Context {
  inner: Mutex<ContextInner>
}

impl Context {
  pub fn new() -> Self {
    return Self {
      inner: Mutex::new(ContextInner {
        root_set_size: 0,
        root_set: LinkedList::new(RootEntryAdapter::new())
      })
    };
  }
  
  pub fn for_each_root(&self, mut iterator: impl FnMut(&RootEntry)) {
    let inner = self.inner.lock().unwrap();
    for entry in &inner.root_set {
      iterator(entry);
    }
  }
}

pub struct ContextHandle<'a> {
  ctx: Arc<Context>,
  obj_manager_ctx: ObjectManagerContextHandle<'a>,
  owner: &'a Heap
}

// ContextHandle will only stays at current thread
impl !Sync for ContextHandle<'_> {}
impl !Send for ContextHandle<'_> {}

pub struct RootRef<'a, T: Any + Send + Sync + 'static> {
  entry_ref: *const RootEntry<'a>,
  phantom: PhantomData<&'a T>
}

// RootRef will only stays at current thread
impl<T> !Sync for RootRef<'_, T> {}
impl<T> !Send for RootRef<'_, T> {}

impl<'a, T: Any + Send + Sync + 'static> RootRef<'a, T> {
  pub fn borrow_inner(&self) -> &T {
    return unsafe { (*self.entry_ref).obj.borrow_inner().unwrap() };
  }
}

impl<'a> ContextHandle<'a> {
  pub(super) fn new(owner: &'a Heap, obj_manager_ctx: ObjectManagerContextHandle<'a>, ctx: Arc<Context>) -> Self {
    return Self {
      ctx,
      owner,
      obj_manager_ctx
    };
  }
  
  pub fn alloc<T: Any + Sync + Send + 'static>(&self, initer: impl FnOnce() -> T) -> RootRef<'a, T> {
    let gc_lock_cookie = self.owner.gc_lock.read().unwrap();
    
    let mut inner = self.ctx.inner.lock().unwrap();
    inner.root_set.push_front(Box::new(RootEntry {
      link: LinkedListLink::new(),
      
      // SAFETY: The recently allocated object can't be deallocated
      // by the GC concurrently, it is protected by GC lock locked earlier
      // therefore GC can't concurrently deallocated by accident
      obj: unsafe { &mut *self.obj_manager_ctx.alloc(initer) }
    }));
    inner.root_set_size += 1;
    
    // Allow GC to run again
    drop(gc_lock_cookie);
    return RootRef {
      entry_ref: inner.root_set.front().get().unwrap() as *const RootEntry,
      phantom: PhantomData {}
    };
  }
}

impl Drop for ContextHandle<'_> {
  fn drop(&mut self) {
    // Remove context belonging to current thread
    self.owner.contexts.lock().unwrap().remove(&thread::current().id());
  }
}

