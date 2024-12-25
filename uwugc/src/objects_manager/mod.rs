use std::{alloc::Layout, any::TypeId, cell::UnsafeCell, collections::HashMap, ptr::{self, NonNull}, sync::{atomic::{AtomicPtr, AtomicUsize, Ordering}, Arc}, thread::{self, ThreadId}};
use crate::allocator::HeapAlloc;
use meta_word::MetaWord;
use parking_lot::{Mutex, RwLock};

use context::LocalObjectsChain;
pub use context::Handle;
use portable_atomic::AtomicBool;

use crate::{descriptor::Descriptor, gc::GCExclusiveLockCookie, ObjectLikeTrait};

mod meta_word;
mod context;

#[derive(Debug)]
pub struct AllocError;

#[repr(align(4))]
pub struct Object {
  next: UnsafeCell<*mut Object>,
  
  // Containing metadata about this object compressed into
  // single machine word
  meta_word: MetaWord,
  
  // Data can only contain owned structs
  data: Box<dyn ObjectLikeTrait>
}

// SAFETY: 'next' pointer for majority of its lifetime
// won't be changed out of few important spots which can
// safetly has &mut on the Object
unsafe impl Sync for Object {}
unsafe impl Send for Object {}

impl Object {
  pub fn new<A: HeapAlloc, T: ObjectLikeTrait, F: FnOnce() -> T>(owner: &ObjectManager<A>, initializer: F, descriptor_obj_ptr: Option<NonNull<Object>>) -> NonNull<Object> {
    let ptr = ptr::from_mut(Box::leak(Box::new(Object {
      data: Box::new(initializer()),
      next: UnsafeCell::new(ptr::null_mut()),
      
      // SAFETY: Caller ensured object pointer is correct and GC ensures
      // that the object pointer to descriptor remains valid as long as
      // there are users of it
      meta_word: unsafe { MetaWord::new(descriptor_obj_ptr, Self::compute_new_object_mark_bit(owner)) }
    })));
    
    // SAFETY: Just allocated the pointer
    unsafe { NonNull::new_unchecked(ptr) }
  }
  
  pub fn get_descriptor_obj_ptr(&self) -> Option<NonNull<Object>> {
    self.meta_word.get_descriptor_obj_ptr()
  }
  
  // SAFETY: Caller has to ensure 'obj' is valid object pointer
  pub unsafe fn get_raw_ptr_to_data(obj: NonNull<Self>) -> NonNull<()> {
    // LOOKING at downcast_ref_unchecked method in dyn Any
    // it looked like &dyn Any can be casted to pointer to
    // T directly therefore can be casted to get untyped
    // pointer to underlying data T
    // SAFETY: Caller ensured 'obj' is valid object pointer
    let ptr = Box::as_ptr(unsafe { &obj.as_ref().data });
    // SAFETY: 'ptr' guarantee to be non null because Box contains pointer to the
    // data
    unsafe { NonNull::new_unchecked(ptr.cast_mut().cast::<()>()) }
  }
  
  fn is_descriptor(&self) -> bool {
    self.meta_word.is_descriptor()
  }
  
  fn get_descriptor(&self) -> &Descriptor {
    self.meta_word.get_descriptor()
  }
  
  pub fn trace(&self, tracer: impl FnMut(&portable_atomic::AtomicPtr<Object>)) {
    // SAFETY: The safety that descriptor is the one needed is enforced by
    // type system and unsafe contract of the getting descriptor for a type
    unsafe {
      self.get_descriptor().trace(Self::get_raw_ptr_to_data(NonNull::from_ref(self)), tracer);
    }
  }
  
  pub fn unset_mark_bit<A: HeapAlloc>(&self, owner: &ObjectManager<A>) {
    let marked_bit_value = owner.marked_bit_value.load(Ordering::Relaxed);
    self.meta_word.swap_mark_bit(!marked_bit_value);
  }
  
  // Return if object was unmark or marked
  pub fn set_mark_bit<A: HeapAlloc>(&self, owner: &ObjectManager<A>) -> bool {
    let marked_bit_value = owner.marked_bit_value.load(Ordering::Relaxed);
    self.meta_word.swap_mark_bit(marked_bit_value) == marked_bit_value
  }
  
  fn is_marked<A: HeapAlloc>(&self, owner: &ObjectManager<A>) -> bool {
    let mark_bit = self.meta_word.get_mark_bit();
    mark_bit == owner.marked_bit_value.load(Ordering::Relaxed)
  }
  
  fn compute_new_object_mark_bit<A: HeapAlloc>(owner: &ObjectManager<A>) -> bool {
    owner.new_object_mark_value.load(Ordering::Relaxed)
  }
  
  // Calculate a layout containing both the Object and padding
  // necessary to the data and return new layout and offset to
  // data part of object
  pub fn calc_layout(data_layout: &Layout) -> (Layout, usize) {
    Layout::new::<Object>()
      .extend(*data_layout)
      .unwrap()
  }
}

pub struct ObjectManager<A: HeapAlloc> {
  head: AtomicPtr<Object>,
  used_size: AtomicUsize,
  contexts: Mutex<HashMap<ThreadId, Arc<LocalObjectsChain>>>,
  max_size: usize,
  #[expect(dead_code)]
  alloc: Arc<A>,
  
  // Descriptors are plain GC objects
  descriptor_cache: RwLock<HashMap<TypeId, NonNull<Object>>>,
  
  // Used to prevent concurrent creation of sweeper, as at the
  // moment two invocation of it going to fight with each other
  sweeper_protect_mutex: Mutex<()>,
  
  // What bit to initialize mark_bit to
  new_object_mark_value: AtomicBool,
  
  // What bit value to consider an object to be marked
  marked_bit_value: AtomicBool,
}

// SAFETY: It is safe to send *const Object to other thread
// because descriptor_cache meant to just cache and GC will
// remove entry once its unused
unsafe impl<A: HeapAlloc> Sync for ObjectManager<A> {}
unsafe impl<A: HeapAlloc> Send for ObjectManager<A> {}

pub struct Sweeper<'a, A: HeapAlloc> {
  owner: &'a ObjectManager<A>,
  saved_chain: Option<*mut Object>
}

impl<A: HeapAlloc> ObjectManager<A> {
  pub fn new(allocator: A, max_size: usize) -> Self {
    Self {
      head: AtomicPtr::new(ptr::null_mut()),
      used_size: AtomicUsize::new(0),
      contexts: Mutex::new(HashMap::new()),
      sweeper_protect_mutex: Mutex::new(()),
      marked_bit_value: AtomicBool::new(true),
      new_object_mark_value: AtomicBool::new(false),
      descriptor_cache: RwLock::new(HashMap::new()),
      alloc: Arc::new(allocator),
      max_size
    }
  }
  
  pub fn flip_marked_bit_value(&self) {
    self.marked_bit_value.fetch_not(Ordering::Relaxed);
  }
  
  pub fn flip_new_marked_bit_value(&self) {
    self.new_object_mark_value.fetch_not(Ordering::Relaxed);
  }
  
  // SAFETY: Caller ensures that 'start' and 'end' is valid Object
  // and also valid chain
  pub(super) unsafe fn add_chain_to_list(&self, start: *const Object, end: *const Object) {
    // NOTE: Relaxed ordering because don't need to access next pointer of the 'head'
    let mut current_head = self.head.load(Ordering::Relaxed);
    loop {
      // Modify 'end' object's 'next' field so it connects to current head 
      // SAFETY: Caller responsibility that 'end' is valid
      let end = unsafe { &*end };
      // SAFETY: The 'next' field in 'end' will ever be only modified
      // by this and caller ensures that 'start' and 'end' is independent
      // chain owned by caller so 'end' is only modified by current thread
      // and won't be accessed to other until its visible by next compare exchange
      unsafe { *end.next.get() = current_head }
      
      // Change head to point to 'start' of chain
      // NOTE: Relaxed failure ordering because don't need to access the 'next' pointer in the head
      // NOTE: Release success ordering because potential changes made by the caller
      // to chain of objects should be visible to other threads now 
      match self.head.compare_exchange_weak(current_head, start.cast_mut(), Ordering::Release, Ordering::Relaxed) {
        Ok(_) => break,
        Err(new_next) => current_head = new_next
      }
    }
  }
  
  // SAFETY: 'obj' must be valid pointer to object
  // which is mutably owned (or there is no other users)
  unsafe fn dealloc(&self, obj: *mut Object) {
    // SAFETY: Caller already ensure 'obj' is valid pointer
    let obj = unsafe { &mut *obj };
    let total_size = Object::calc_layout(&obj.get_descriptor().layout).0.size();
    
    // SAFETY: Caller ensured that 'obj' pointer is only user left
    // and safe to be deallocated
    unsafe { drop(Box::from_raw(obj)) };
    self.used_size.fetch_sub(total_size, Ordering::Relaxed);
  }
  
  pub fn create_context(&self) -> Handle<A> {
    let mut contexts = self.contexts.lock();
    let ctx = contexts.entry(thread::current().id())
      .or_insert(Arc::new(LocalObjectsChain::new()))
      .clone();
    
    Handle::new(ctx, self)
  }
  
  pub fn create_sweeper(&self, _mutator_lock_cookie: &mut GCExclusiveLockCookie) -> Sweeper<A> {
    // SAFETY: Already got exclusive GC lock
    unsafe { self.create_sweeper_impl() }
  }
  
  // Unsafe because this need that the ctx isnt modified concurrently
  // its being protected by exclusive GC lock by design
  unsafe fn create_sweeper_impl(&self) -> Sweeper<A> {
    let sweeper_lock_guard = self.sweeper_protect_mutex.lock();
    
    // Flush all contexts' local chain to global before sweeping
    for ctx in self.contexts.lock().values() {
      // SAFETY: 'self' owns the objects chain and is being protected
      // from concurrent modification both by locking the 'contexts'
      // and being in exclusive GC (which is assured by the caller)
      //
      // Because this need both being protected by 'contexts' and
      // exclusive GC lock to be safe
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
    sweeper
  }
  
  pub fn get_usage(&self) -> usize {
    self.used_size.load(Ordering::Relaxed)
  }
  
  // SAFETY: Caller ensures that descriptor which is in use
  // be marked and ensure the objects corresponding to descriptor
  // is not being GC'ed away and not being concurrently marked
  pub unsafe fn prune_descriptor_cache(&self) {
    self.descriptor_cache.write().retain(|_, &mut desc| {
      // SAFETY: Caller ensures that object pointer to the descriptor
      // still valid
      unsafe { desc.as_ref().is_marked(self) }
    });
  }
}

impl<A: HeapAlloc> Drop for ObjectManager<A> {
  fn drop(&mut self) {
    assert!(self.get_usage() == 0, "GC did not free everything during shutdown!");
  }
}

impl<A: HeapAlloc> Sweeper<'_, A> {
  // Sweeps dead objects and consume this sweeper
  // SAFETY: Caller must ensure live objects actually
  // marked!
  pub unsafe fn sweep_and_reset_mark_flag(mut self) {
    let mut live_objects: *mut Object = ptr::null_mut();
    let mut last_live_objects: *mut Object = ptr::null_mut();
    let mut next_ptr = self.saved_chain.take().unwrap();
    
    // List of objects which its deallocation be deferred.
    // This list only contains dead descriptors which has to
    // be alive during initial dealloc because descriptor could
    // be used during dealloc.
    let mut deferred_dealloc_list: *mut Object = ptr::null_mut();
    
    while !next_ptr.is_null() {
      let current_ptr = next_ptr;
      
      // SAFETY: Sweeper "owns" the individual object's 'next' field
      unsafe {
        // Get pointer to next, and disconnect current object from chain
        next_ptr = *(*current_ptr).next.get();
        *(*current_ptr).next.get() = ptr::null_mut();
      }
      
      // SAFETY: 'current' is valid because its leaked
      if unsafe { !(*current_ptr).is_marked(self.owner) } {
        // SAFETY: 'current' is valid because its leaked
        let current = unsafe { &*current_ptr };
        
        // It is descriptor object, defer it to deallocate later
        if current.is_descriptor() {
          if deferred_dealloc_list.is_null() {
            deferred_dealloc_list = current_ptr;
          } else {
            // SAFETY: Sweeper "owns" the individual object's 'next' field
            unsafe { *current.next.get() = deferred_dealloc_list };
            deferred_dealloc_list = current_ptr;
          }
          
          // Defer deallocation beacuse descriptor object might
          // be needed during deallocation and could cause
          // use-after-free in deallocation path if deallocated
          continue;
        }
        
        // SAFETY: *const can be safely converted to *mut as unmarked object
        // mean mutator has no way accesing it
        unsafe { self.owner.dealloc(current_ptr.cast()) };
        continue;
      }
      
      // SAFETY: 'current' is valid because its leaked
      let current = unsafe { &*current_ptr };
      
      // First live object, init the chain
      if live_objects.is_null() {
        live_objects = current_ptr;
        last_live_objects = current_ptr;
      } else {
        // Append current object to list of live objects
        // SAFETY: Sweeper "owns" the individual object's 'next' field
        unsafe { *current.next.get() = live_objects };
        live_objects = current_ptr;
      }
    }
    
    // Now dealloc the deferred deallocs
    next_ptr = deferred_dealloc_list;
    while !next_ptr.is_null() {
      let current = next_ptr;
      
      // SAFETY: Sweeper "owns" the individual object's 'next' field
      // and Sweeper hasn't deallocated it
      next_ptr = unsafe { *(*current).next.get() };
      
      // SAFETY: *const can be safely converted to *mut as unmarked object
      // mean mutator has no way accesing it
      unsafe { self.owner.dealloc(current) };
    }
    
    // There are no living objects
    if live_objects.is_null() {
      // If there no live objects, the last can't exist
      assert!(last_live_objects.is_null());
      return;
    }
    
    // If there are live objects, 'last_live_objects' can't be null
    assert!(!last_live_objects.is_null());
    
    // SAFETY: Objects are alive and a valid singly linked chain
    unsafe {
      self.owner.add_chain_to_list(live_objects, last_live_objects);
    }
  }
}

impl<A: HeapAlloc> Drop for Sweeper<'_, A> {
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
    assert!(self.saved_chain.is_none() || thread::panicking(), "Sweeper must not be dropped before sweep is called!");
  }
}


