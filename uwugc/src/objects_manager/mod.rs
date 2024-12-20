use std::{collections::HashMap, ops::{Deref, DerefMut}, ptr, sync::{atomic::{AtomicPtr, AtomicUsize, Ordering}, Arc}, thread::{self, ThreadId}};
use parking_lot::Mutex;

use context::{ContextHandle, LocalObjectsChain};
use portable_atomic::AtomicBool;

use crate::descriptor::Descriptor;

pub mod context;

// What the data type need to implement before it is
// adequate for GC system to use
// NOTE: This type is considered to be part of public API
pub trait ObjectLikeTrait: 'static {
}

impl<T: 'static> ObjectLikeTrait for T {
}

#[derive(Debug)]
pub struct AllocError;

// Another struct layer of indirection
// to force implement Send + Sync as its
// not required to Send + Sync and GC
// only needs to safe sync access to the
// reference fields
pub struct ObjectDataContainer {
  data: Box<dyn ObjectLikeTrait>
}

// SAFETY: The Send + Sync is forced because
// the GC designed for multiple threads usage
// and the Send + Sync for the data contained
// by it is enforced at higher level not here
unsafe impl Send for ObjectDataContainer {}
unsafe impl Sync for ObjectDataContainer {}

impl ObjectDataContainer {
  pub fn new(data: Box<dyn ObjectLikeTrait>) -> Self {
    return Self {
      data
    };
  }
}

impl Deref for ObjectDataContainer {
  type Target = Box<dyn ObjectLikeTrait>;
  
  fn deref(&self) -> &Self::Target {
    return &self.data;
  }
}

impl DerefMut for ObjectDataContainer {
  fn deref_mut(&mut self) -> &mut Self::Target {
    return &mut self.data;
  }
}

pub struct Object {
  next: AtomicPtr<Object>,
  // WARNING: Do not rely on this, always use is_marked
  // function, the 'marked' meaning on this always changes
  marked: AtomicBool,
  total_size: usize,
  descriptor: Option<&'static Descriptor>,
  
  // Data can only contain owned structs
  data: ObjectDataContainer
}

impl Object {
  fn get_raw_ptr_to_data(&self) -> *const () {
    // LOOKING at downcast_ref_unchecked method in dyn Any
    // it looked like &dyn Any can be casted to pointer to
    // T directly therefore can be casted to get untyped
    // pointer to underlying data T
    //
    // TODO: Is this correct?
    return self.data.as_ref() as *const dyn ObjectLikeTrait as *const ();
  }
  
  pub fn trace(&self, tracer: impl FnMut(&portable_atomic::AtomicPtr<Object>)) {
    if self.descriptor.is_some() {
      // SAFETY: The safety that descriptor is the one needed is enforced by
      // type system and unsafe contract of the getting descriptor for a type
      unsafe {
        self.descriptor.unwrap().trace(self.get_raw_ptr_to_data(), tracer);
      }
    }
  }
  
  pub fn unset_mark_bit(&self, owner: &ObjectManager) {
    let marked_bit_value = owner.marked_bit_value.load(Ordering::Relaxed);
    self.marked.store(!marked_bit_value, Ordering::Relaxed);
  }
  
  // Return if object was unmark or marked
  pub fn set_mark_bit(&self, owner: &ObjectManager) -> bool {
    let marked_bit_value = owner.marked_bit_value.load(Ordering::Relaxed);
    return self.marked.swap(marked_bit_value, Ordering::Relaxed) == marked_bit_value;
  }
  
  fn is_marked(&self, owner: &ObjectManager) -> bool {
    let mark_bit = self.marked.load(Ordering::Relaxed);
    return mark_bit == owner.marked_bit_value.load(Ordering::Relaxed);
  }
  
  fn compute_new_object_mark_bit(owner: &ObjectManager) -> bool {
    return owner.new_object_mark_value.load(Ordering::Relaxed);
  }
}

pub struct ObjectManager {
  head: AtomicPtr<Object>,
  used_size: AtomicUsize,
  contexts: Mutex<HashMap<ThreadId, Arc<LocalObjectsChain>>>,
  max_size: usize,
  
  // Used to prevent concurrent creation of sweeper, as at the
  // moment two invocation of it going to fight with each other
  sweeper_protect_mutex: Mutex<()>,
  
  // What bit to initialize mark_bit to
  new_object_mark_value: AtomicBool,
  
  // What bit value to consider an object to be marked
  marked_bit_value: AtomicBool,
}

pub struct Sweeper<'a> {
  owner: &'a ObjectManager,
  saved_chain: Option<*mut Object>
}

impl Object {
  // SAFETY: Caller must ensure the T is correct type for given
  // object
  pub unsafe fn borrow_inner<T: ObjectLikeTrait>(&self) -> &T {
    // SAFETY: Caller already provided correct T
    return unsafe { &*(self.data.as_ref() as *const _ as *const T) };
  }
  
  // SAFETY: Caller must ensure the T is correct type for given
  // object
  pub unsafe fn borrow_inner_mut<T: ObjectLikeTrait>(&mut self) -> &mut T {
    // SAFETY: Caller already provided correct T
    return unsafe { &mut *(self.data.as_mut() as *const _ as *mut T) };
  }
}

impl ObjectManager {
  pub fn new(max_size: usize) -> Self {
    return Self {
      head: AtomicPtr::new(ptr::null_mut()),
      used_size: AtomicUsize::new(0),
      contexts: Mutex::new(HashMap::new()),
      sweeper_protect_mutex: Mutex::new(()),
      marked_bit_value: AtomicBool::new(true),
      new_object_mark_value: AtomicBool::new(false),
      max_size
    };
  }
  
  pub fn flip_marked_bit_value(&self) {
    self.marked_bit_value.fetch_not(Ordering::Relaxed);
  }
  
  pub fn flip_new_marked_bit_value(&self) {
    self.new_object_mark_value.fetch_not(Ordering::Relaxed);
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
    let mut contexts = self.contexts.lock();
    let ctx = contexts.entry(thread::current().id())
      .or_insert(Arc::new(LocalObjectsChain::new()))
      .clone();
    
    return ContextHandle::new(ctx, self);
  }
  
  pub fn create_sweeper(&self) -> Sweeper {
    let sweeper_lock_guard = self.sweeper_protect_mutex.lock();
    
    // Flush all contexts' local chain to global before sweeping
    for ctx in self.contexts.lock().values() {
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
      
      if !current.is_marked(self.owner) {
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


