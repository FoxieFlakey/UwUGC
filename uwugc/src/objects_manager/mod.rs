use std::{alloc::Layout, any::TypeId, cell::UnsafeCell, collections::HashMap, mem::MaybeUninit, ops::{Add, AddAssign, Sub, SubAssign}, ptr::{self, NonNull}, sync::{atomic::{AtomicPtr, AtomicUsize, Ordering}, Arc}, thread::{self, ThreadId}};
use crate::{allocator::HeapAlloc, ReferenceType};
use meta_word::{MetaWord, ObjectMetadata};
use parking_lot::{Mutex, RwLock};

use context::LocalObjectsChain;
pub use context::Handle;
use portable_atomic::AtomicBool;

use crate::{gc::GCExclusiveLockCookie, ObjectLikeTraitInternal};

mod meta_word;
mod context;

#[derive(Debug)]
pub struct AllocError;

#[repr(align(4))]
pub struct Object {
  next: UnsafeCell<*mut Object>,
  
  // Containing metadata about this object compressed into
  // single machine word
  meta_word: MetaWord
}

// SAFETY: 'next' pointer for majority of its lifetime
// won't be changed out of few important spots which can
// safetly has &mut on the Object
unsafe impl Sync for Object {}
unsafe impl Send for Object {}

pub enum NewPodError {
  UnsuitableObject,
  AllocError(AllocError)
}

impl Object {
  // SAFETY: Initializers must properly initialize T, before returning!
  pub fn new<A: HeapAlloc, T: ObjectLikeTraitInternal, F: FnOnce(&mut MaybeUninit<T>)>(owner: &ObjectManager<A>, initializer: F, descriptor_obj_ptr: Option<NonNull<Object>>) -> Result<NonNull<Object>, AllocError> {
    // SAFETY: Caller ensured object pointer is correct and GC ensures
    // that the object pointer to descriptor remains valid as long as
    // there are users of it
    let meta_word = unsafe { MetaWord::new(descriptor_obj_ptr, Self::compute_new_object_mark_bit(owner)) };
    Self::new_common(owner, initializer, meta_word)
  }
  
  // SAFETY: Initializers must properly initialize the array, before returning!
  pub unsafe fn new_array<A: HeapAlloc, Ref: ReferenceType, F: FnOnce(&mut MaybeUninit<[Ref; LEN]>), const LEN: usize>(owner: &ObjectManager<A>, initializer: F) -> Result<NonNull<Object>, AllocError> {
    let meta_word = MetaWord::new_array(LEN, Self::compute_new_object_mark_bit(owner));
    
    // Oversized arrays consider out of memory because in practical case the limits
    // imposed by it is actually many many time larger than what available like on 64-bit
    // well its maximum is 2^61 entries and modern CPUs at the time of writing only can
    // address somewhere around 2^48 bytes while on 32-bit system its 2^29 entries but
    // it needs 2^31 bytes and that basically half of what can be addressed by it and
    // "that big of chunk of memory" is really scarce on 32-bit userspaces
    Self::new_common(owner, initializer, meta_word.map_err(|_| AllocError)?)
  }
  
  // Placeholder for future special support for POD to
  // fit layout inside meta word, removing need for new
  // descriptors for suitable objects (size and alignment
  // smaller than the limit of what can be stored in meta word)
  //
  // SAFETY: Caller has to make sure that T has no GC references in it
  // and also T doesn't have any special drop code to be ran like a structure
  // containing only primitives (Descendants of GCRefRaw<T> doesn't  have any
  // drop code to be ran) and initialize must properly inialize T before
  // returning
  pub unsafe fn new_pod<A: HeapAlloc, T: ObjectLikeTraitInternal, F: FnOnce(&mut MaybeUninit<T>)>(owner: &ObjectManager<A>, layout: Layout, initializer: F) -> Result<NonNull<Object>, NewPodError> {
    let meta_word = MetaWord::new_pod(layout, Self::compute_new_object_mark_bit(owner))
      .map_err(|_| NewPodError::UnsuitableObject)?;
    
    Self::new_common(owner, initializer, meta_word)
      .map_err(NewPodError::AllocError)
  }
  
  fn new_common<A: HeapAlloc, T: ObjectLikeTraitInternal, F: FnOnce(&mut MaybeUninit<T>)>(owner: &ObjectManager<A>, initializer: F, meta_word: MetaWord) -> Result<NonNull<Object>, AllocError> {
    let header = Object {
      next: UnsafeCell::new(ptr::null_mut()),
      meta_word
    };
    
    let (object_layout, data_offset) = header.get_object_and_data_layout();
    
    owner.alloc.allocate(object_layout)
      .map(|new_obj| {
        let new_obj = new_obj.cast::<()>();
        
        // SAFETY: Already calculated proper offset for getting
        // to data region and just allocated the memory
        unsafe {
          let header_region = new_obj.cast::<Object>().as_ptr();
          let data_region = new_obj.byte_add(data_offset).cast::<T>().as_ptr();
          
          // Initialize the header region
          header_region.write(header);
          // Initialize the data region
          // SAFETY: The data_region currently uninitialized
          initializer(data_region.as_uninit_mut().unwrap());
        }
        
        new_obj.cast::<Object>()
      })
      .map_err(|_| AllocError /* This just replace allocator API's AllocError with custom one */ )
  }
  
  // SAFETY: Caller has to ensure 'obj' is valid object pointer
  pub unsafe fn get_raw_ptr_to_data(obj: NonNull<Self>) -> NonNull<()> {
    // SAFETY: Caller ensured obj is valid pointer
    let header = unsafe { obj.as_ref() };
    let (_, data_offset) = header.get_object_and_data_layout();
    
    // SAFETY: Already calculated correct offset for it
    // and constructor allocated suitable region for required
    // layout
    unsafe { obj.byte_add(data_offset).cast() }
  }
  
  // SAFETY: Caller has to make sure that 'obj' is valid object pointer
  pub unsafe fn trace(obj_ptr: NonNull<Self>, mut tracer: impl FnMut(Option<NonNull<Object>>)) {
    // SAFETY: Caller also make sure 'obj' is valid
    let obj = unsafe { obj_ptr.as_ref() };
    match obj.meta_word.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => {
        // Trace the descriptor too
        tracer(meta.get_descriptor_obj());
        
        // SAFETY: The safety that descriptor is the one needed is enforced by
        // type system and unsafe contract of the getting descriptor for a type
        unsafe { meta.get_descriptor().trace(Self::get_raw_ptr_to_data(obj_ptr), |field| tracer(NonNull::new(field.load(Ordering::Relaxed)))) };
      },
      ObjectMetadata::ReferenceArray(meta) => {
        // SAFETY: obj_ptr is ensured to be safe by caller
        let array_ptr = unsafe { Self::get_raw_ptr_to_data(obj_ptr).cast::<AtomicPtr<Object>>() };
        for i in 0..meta.get_array_len() {
          // SAFETY: Reference array are guaranteed to be type of [AtomicPtr<Object>; N]
          // and is guaranteed tightly packed
          // Source: https://doc.rust-lang.org/nomicon/repr-rust.html
          let item = unsafe { (*array_ptr.add(i).as_ptr()).load(Ordering::Relaxed) };
          tracer(NonNull::new(item));
        }
      },
      
      // POD data doesnt have any GC pointers
      // so do nothing here
      ObjectMetadata::PodData(_) => ()
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
  
  pub fn get_object_and_data_layout(&self) -> (Layout, usize) {
    let data_layout = match self.meta_word.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => meta.get_descriptor().layout,
      ObjectMetadata::ReferenceArray(meta) => {
        // Can safely assume [AtomicPtr<Object>; N] is tightly packed
        // Rust guarantee that
        // source: https://doc.rust-lang.org/nomicon/repr-rust.html
        Layout::new::<AtomicPtr<Object>>()
          .repeat_packed(meta.get_array_len())
          .unwrap()
      },
      ObjectMetadata::PodData(meta) => meta.get_layout()
    };
    
    Self::calc_layout(&data_layout)
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

// NOTE: This is part of public API
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub struct Stats {
  pub alloc_attempt_bytes: usize,
  pub alloc_success_bytes: usize,
  pub dealloc_bytes: usize
}

impl SubAssign for Stats {
  fn sub_assign(&mut self, rhs: Self) {
    *self = *self + rhs;
  }
}

impl AddAssign for Stats {
  fn add_assign(&mut self, rhs: Self) {
    *self = *self + rhs;
  }
}

impl Sub for Stats {
  type Output = Self;
  
  fn sub(self, rhs: Self) -> Self::Output {
    Self {
      alloc_attempt_bytes: self.alloc_attempt_bytes - rhs.alloc_attempt_bytes,
      alloc_success_bytes: self.alloc_success_bytes - rhs.alloc_success_bytes,
      dealloc_bytes: self.dealloc_bytes - rhs.dealloc_bytes
    }
  }
}

impl Add for Stats {
  type Output = Self;
  
  fn add(self, rhs: Self) -> Self::Output {
    Self {
      alloc_attempt_bytes: self.alloc_attempt_bytes + rhs.alloc_attempt_bytes,
      alloc_success_bytes: self.alloc_success_bytes + rhs.alloc_success_bytes,
      dealloc_bytes: self.dealloc_bytes + rhs.dealloc_bytes
    }
  }
}

pub struct ObjectManager<A: HeapAlloc> {
  head: AtomicPtr<Object>,
  used_size: AtomicUsize,
  
  // Bytes that object manager tried to allocates
  // which may or may not failed
  lifetime_alloc_attempt_bytes: AtomicUsize,
  // Bytes which object manager successfully
  // allocated
  lifetime_alloc_success_bytes: AtomicUsize,
  // Bytes which is deallocated
  lifetime_dealloc_bytes: AtomicUsize,
  
  contexts: Mutex<HashMap<ThreadId, Arc<LocalObjectsChain>>>,
  max_size: usize,
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
      lifetime_alloc_attempt_bytes: AtomicUsize::new(0),
      lifetime_alloc_success_bytes: AtomicUsize::new(0),
      lifetime_dealloc_bytes: AtomicUsize::new(0),
      contexts: Mutex::new(HashMap::new()),
      sweeper_protect_mutex: Mutex::new(()),
      marked_bit_value: AtomicBool::new(true),
      new_object_mark_value: AtomicBool::new(false),
      descriptor_cache: RwLock::new(HashMap::new()),
      alloc: Arc::new(allocator),
      max_size
    }
  }
  
  pub fn get_lifetime_stats(&self) -> Stats {
    Stats {
      alloc_attempt_bytes: self.lifetime_alloc_attempt_bytes.load(Ordering::Relaxed),
      alloc_success_bytes: self.lifetime_alloc_success_bytes.load(Ordering::Relaxed),
      dealloc_bytes: self.lifetime_dealloc_bytes.load(Ordering::Relaxed)
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
  unsafe fn dealloc(&self, obj: NonNull<Object>) {
    // SAFETY: Caller already ensure 'obj' is valid pointer
    let obj_ref = unsafe { obj.as_ref() };
    let drop_helper = match obj_ref.meta_word.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => Some(meta.get_descriptor().drop_helper),
      
      // Reference array, does not need to call its drop function
      // and
      // POD data does not have destructor (or drop)
      // because type which does not do anything in drop code is no-op
      // and POD is specifically exist for those types which has nothing
      // to run in its drop code recursively too following its fields
      ObjectMetadata::ReferenceArray(_) | ObjectMetadata::PodData(_) => None,
    };
    let (layout, data_offset) = obj_ref.get_object_and_data_layout();
    
    // SAFETY: Caller already ensure 'obj' is valid pointer
    // and already calculate the offset correctly
    unsafe {
      // Drop the header
      ptr::drop_in_place(obj.as_ptr());
      
      // Drop the data itself with helper, because cannot
      // know the type at this point so ask helper to do it
      if let Some(func) = drop_helper {
        func(obj.cast::<()>().byte_add(data_offset).as_ptr());
      }
    }
    
    // SAFETY: Caller ensured that 'obj' pointer is only user left
    // and safe to be deallocated
    unsafe { self.alloc.deallocate(obj.cast(), layout); };
    self.used_size.fetch_sub(layout.size(), Ordering::Relaxed);
    self.lifetime_dealloc_bytes.fetch_add(layout.size(), Ordering::Relaxed);
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

pub struct SweeperStatistic {
  pub total_bytes: usize,
  pub live_bytes: usize,
  pub dead_bytes: usize,
  
  pub total_objects: usize,
  pub live_objects: usize,
  pub dead_objects: usize
}

impl<A: HeapAlloc> Sweeper<'_, A> {
  // Sweeps dead objects and consume this sweeper
  // SAFETY: Caller must ensure live objects actually
  // marked!
  pub unsafe fn sweep_and_reset_mark_flag(mut self) -> SweeperStatistic {
    let mut stats = SweeperStatistic {
      dead_bytes: 0,
      live_bytes: 0,
      total_bytes: 0,
      
      dead_objects: 0,
      live_objects: 0,
      total_objects: 0
    };
    
    let mut live_objects: *mut Object = ptr::null_mut();
    let mut last_live_objects: *mut Object = ptr::null_mut();
    let mut next_ptr = self.saved_chain.take().unwrap();
    
    // List of objects which its deallocation be deferred.
    // This list only contains dead descriptors which has to
    // be alive during initial dealloc because descriptor could
    // be used during dealloc.
    let mut deferred_dealloc_list: *mut Object = ptr::null_mut();
    
    while !next_ptr.is_null() {
      // SAFETY: Already checked before that it is nonnull
      let current_ptr = unsafe { NonNull::new_unchecked(next_ptr) };
      
      // SAFETY: 'current' is valid because its leaked
      let current = unsafe { current_ptr.as_ref() };
      
      // SAFETY: Sweeper "owns" the individual object's 'next' field
      unsafe {
        // Get pointer to next, and disconnect current object from chain
        next_ptr = *current.next.get();
        *current.next.get() = ptr::null_mut();
      }
      
      let cur_size = current.get_object_and_data_layout().0.size();
      stats.total_bytes += cur_size;
      stats.total_objects += 1;
      
      if !current.is_marked(self.owner) {
        stats.dead_bytes += cur_size;
        stats.dead_objects += 1;
        
        // It is descriptor object, defer it to deallocate later
        if let ObjectMetadata::Ordinary(meta) = current.meta_word.get_object_metadata() {
          if meta.is_descriptor() {
            if deferred_dealloc_list.is_null() {
              deferred_dealloc_list = current_ptr.as_ptr();
            } else {
              // SAFETY: Sweeper "owns" the individual object's 'next' field
              unsafe { *current.next.get() = deferred_dealloc_list };
              deferred_dealloc_list = current_ptr.as_ptr();
            }
            
            // Defer deallocation beacuse descriptor object might
            // be needed during deallocation and could cause
            // use-after-free in deallocation path if deallocated
            continue;
          }
        }
        
        // SAFETY: *const can be safely converted to *mut as unmarked object
        // mean mutator has no way accesing it
        unsafe { self.owner.dealloc(current_ptr.cast()) };
        continue;
      }
      
      stats.live_bytes += cur_size;
      stats.live_objects += 1;
      
      // First live object, init the chain
      if live_objects.is_null() {
        live_objects = current_ptr.as_ptr();
        last_live_objects = current_ptr.as_ptr();
      } else {
        // Append current object to list of live objects
        // SAFETY: Sweeper "owns" the individual object's 'next' field
        unsafe { *current.next.get() = live_objects };
        live_objects = current_ptr.as_ptr();
      }
    }
    
    // Now dealloc the deferred deallocs
    next_ptr = deferred_dealloc_list;
    while !next_ptr.is_null() {
      // SAFETY: Already checked that pointer is non null
      let current = unsafe { NonNull::new_unchecked(next_ptr) };
      
      // SAFETY: Sweeper "owns" the individual object's 'next' field
      // and Sweeper hasn't deallocated it
      next_ptr = unsafe { *current.as_ref().next.get() };
      
      // SAFETY: *const can be safely converted to *mut as unmarked object
      // mean mutator has no way accesing it
      unsafe { self.owner.dealloc(current) };
    }
    
    // There are no living objects
    if live_objects.is_null() {
      // If there no live objects, the last can't exist
      assert!(last_live_objects.is_null());
      return stats;
    }
    
    // If there are live objects, 'last_live_objects' can't be null
    assert!(!last_live_objects.is_null());
    
    // SAFETY: Objects are alive and a valid singly linked chain
    unsafe {
      self.owner.add_chain_to_list(live_objects, last_live_objects);
    }
    
    stats
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


