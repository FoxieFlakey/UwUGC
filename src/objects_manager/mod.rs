use std::{any::Any, collections::HashMap, ptr, sync::{atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering}, Arc, Mutex}, thread::{self, ThreadId}};

use context::LocalObjectsChain;
pub use context::ContextHandle;

mod context;

pub struct Object {
  next: AtomicPtr<Object>,
  marked: AtomicBool,
  total_size: usize,
  
  // Data can only contain owned structs
  data: Box<dyn Any + Send + Sync + 'static>
}

impl Object {
  pub fn mark(&self) {
    self.marked.store(true, Ordering::Relaxed);
  }
}

pub struct ObjectManager {
  head: AtomicPtr<Object>,
  used_size: AtomicUsize,
  contexts: Mutex<HashMap<ThreadId, Arc<LocalObjectsChain>>>,
  
  // Used to prevent concurrent creation of sweeper, as at the
  // moment two invocation of it going to fight with each other
  sweeper_protect_mutex: Mutex<()>
}

pub struct Sweeper<'a> {
  owner: &'a ObjectManager,
  saved_chain: Option<*mut Object>
}

impl Object {
  pub fn borrow_inner<T: Any>(&self) -> Option<&T> {
    return self.data.downcast_ref::<T>();
  }
}

impl ObjectManager {
  pub fn new() -> Self {
    return Self {
      head: AtomicPtr::new(ptr::null_mut()),
      used_size: AtomicUsize::new(0),
      contexts: Mutex::new(HashMap::new()),
      sweeper_protect_mutex: Mutex::new(())
    };
  }
  
  // SAFETY: Caller ensures that 'start' and 'end' is valid Object
  // and also valid chain
  pub(super) unsafe fn add_chain_to_list(&self, start: *mut Object, end: *mut Object) {
    // NOTE: Relaxed ordering because don't need to access data pointed by "head"
    let mut current_head = self.head.load(Ordering::Relaxed);
    loop {
      // Modify 'end' object's 'next' field so it connects to current head 
      // SAFETY: Caller responsibility that 'end' is valid
      unsafe { (*end).next.store(current_head, Ordering::Relaxed) };
      
      // Change head to point to 'start' of chain
      // NOTE: Relaxed failure ordering because don't need to access the pointer in the head
      // NOTE: Release success ordering because previous changes on the chain need to be visible
      // along with modification to the "next" field of the 'end' object
      match self.head.compare_exchange_weak(current_head, start, Ordering::Release, Ordering::Relaxed) {
        Ok(_) => break,
        Err(new_next) => current_head = new_next
      }
    }
  }
  
  fn dealloc(&self, obj: &mut Object) {
    let total_size = obj.total_size;
    // SAFETY: Caller already ensure 'obj' is valid reference
    // because references in Rust must be valid
    unsafe { drop(Box::from_raw(obj)) };
    self.used_size.fetch_sub(total_size, Ordering::Relaxed);
  }
  
  pub fn create_context(&self) -> ContextHandle {
    let mut contexts = self.contexts.lock().unwrap();
    let ctx = contexts.entry(thread::current().id())
      .or_insert(Arc::new(LocalObjectsChain::new()))
      .clone();
    
    return ContextHandle::new(ctx, self);
  }
  
  pub fn create_sweeper(&self) -> Sweeper {
    let sweeper_lock_guard = self.sweeper_protect_mutex.lock().unwrap();
    
    // Flush all contexts' local chain to global before sweeping
    for ctx in self.contexts.lock().unwrap().values() {
      // SAFETY: 'self' owns the objects chain
      unsafe { ctx.flush_to_global(self) };
    }
    
    let sweeper = Sweeper {
      owner: self,
      // Atomically empty the list and get snapshot of current objects
      // at the time of swap, Ordering::Acquire because changes must be
      // visible now
      saved_chain: Some(self.head.swap(ptr::null_mut(), Ordering::Acquire))
    };
    
    drop(sweeper_lock_guard);
    return sweeper;
  }
  
  pub fn get_usage(&self) -> usize {
    return self.used_size.load(Ordering::Relaxed);
  }
}

impl Drop for ObjectManager {
  fn drop(&mut self) {
    // SAFETY: Rust lifetime limit on the Context and lifetime on
    // allocated object reference ensures that any ObjectRef does
    // not live longer than ObjectManager, therefore its safe
    unsafe { self.create_sweeper().sweep_and_reset_mark_flag() };
  }
}

impl Sweeper<'_> {
  // Sweeps dead objects and consume this sweeper
  // SAFETY: Caller must ensure live objects actually
  // marked!
  pub unsafe fn sweep_and_reset_mark_flag(mut self) {
    let mut live_objects: *mut Object = ptr::null_mut();
    let mut last_live_objects: *mut Object = ptr::null_mut();
    let mut iter_current_ptr = self.saved_chain.take().unwrap();
    
    // Run 'predicate' for each object, so can decide whether to deallocate or not
    while iter_current_ptr != ptr::null_mut() {
      let current_ptr = iter_current_ptr;
      
      // Note: Ordering::Acquire because changes in 'next' object need to be visible
      // SAFETY: 'current' is valid because is leaked and only deallocated by current
      // thread and the list only ever appended outside of this method
      let current = unsafe { &mut *current_ptr };
      iter_current_ptr = current.next.load(Ordering::Acquire);
      
      if !current.marked.swap(false, Ordering::Relaxed) {
        // 'predicate' determine that 'current' object is to be deallocated
        self.owner.dealloc(current);
        
        // 'current' reference now is invalid, drop it to let compiler
        // know that it can't be used anymore and as a note to future
        // me UwU
        #[allow(dropping_references)]
        drop(current);
        continue;
      }
      
      // First live object, init the chain
      if live_objects == ptr::null_mut() {
        live_objects = current_ptr;
        last_live_objects = current_ptr;
      } else {
        // Relaxed because changes dont need to be visible yet
        // until the time to make it visible to other threads
        current.next.store(live_objects, Ordering::Relaxed);
        live_objects = current_ptr;
      }
    }
    
    // There are no living objects
    if live_objects == ptr::null_mut() {
      // If there no live objects, the last can't exist
      assert_eq!(last_live_objects, ptr::null_mut());
      return;
    }
    
    // If there are live objects, 'last_live_objects' can't be null
    assert_ne!(last_live_objects, ptr::null_mut());
    
    // SAFETY: Objects are alive and a valid singly linked chain
    unsafe {
      self.owner.add_chain_to_list(live_objects, last_live_objects);
    }
  }
}

impl Drop for Sweeper<'_> {
  fn drop(&mut self) {
    // There no reasonable thing can be done here
    // 1. Execute the sweep anyway -> may leads to memory unsafety in other safe codes
    //    due can't be sure which is in use
    // 2. Dont execute the sweep -> memory leaks! this Sweeper move all objects into it
    // 3. Add back to origin -> Quite not good, because confusing behaviour
    //
    // Leaking memory during panicking is fine because program going to
    // die anyway so leak the objects so it can be used during panicking
    // for maybe some strange codes
    if self.saved_chain.is_some() && !thread::panicking() {
      panic!("Sweeper must not be dropped before sweep is called!");
    }
  }
}


