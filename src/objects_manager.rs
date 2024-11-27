use std::{any::Any, marker::PhantomData, ptr, sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering}};

pub struct Object {
  next: AtomicPtr<Object>,
  marked: AtomicBool,
  
  // Data can only contain owned structs
  data: Box<dyn Any + 'static>
}

pub struct ObjectManager {
  head: AtomicPtr<Object>,
  used_size: AtomicUsize,
  phantom: PhantomData<()>
}

pub struct ObjectRef<'a, T: 'a> {
  obj: &'a mut Object,
  phantom: PhantomData<T>
}

impl<'a, T: 'static> ObjectRef<'a, T> {
  fn new(obj: &'a mut Object) -> Self {
    return Self {
      phantom: PhantomData {},
      obj
    }
  }
  
  pub fn borrow_inner(&self) -> &T {
    return self.obj.data.downcast_ref().unwrap();
  }
  
  pub fn borrow_mut(&mut self) -> &mut T {
    return self.obj.data.downcast_mut().unwrap();
  }
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
      phantom: PhantomData {},
      used_size: AtomicUsize::new(0)
    };
  }
  
  // SAFETY: Caller ensures that 'start' and 'end' is valid Object
  // and also valid chain
  unsafe fn add_chain_to_list(&self, start: *mut Object, end: *mut Object) {
    // NOTE: Relaxed ordering because don't need to access data pointed by "head"
    let mut current_head = self.head.load(Ordering::Relaxed);
    loop {
      // Modify 'end' object's 'next' field so it connects to current head 
      (*end).next.store(current_head, Ordering::Relaxed);
      
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
    // SAFETY: Caller already ensure 'obj' is valid reference
    // because references in Rust must be valid
    unsafe { drop(Box::from_raw(obj)) };
  }
  
  pub fn alloc<'a, T: Any + 'static>(&'a self, func: impl FnOnce() -> T) -> ObjectRef<'a, T> {
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
      self.add_chain_to_list(obj, obj);
    }
    
    let allocated_size = size_of_val(obj) + size_of_val(obj.data.as_ref());
    self.used_size.fetch_add(allocated_size, Ordering::Relaxed);
    return ObjectRef::new(obj);
  }
  
  // Filters alive object to be kept alive
  // if predicate return false, the object
  // is deallocated else kept alive
  pub fn sweep(&self, mut predicate: impl FnMut(&Object) -> bool) {
    let mut live_objects: *mut Object = ptr::null_mut();
    let mut last_live_objects: *mut Object = ptr::null_mut();
    
    // Atomically empty the list and get snapshot of current objects
    // at the time of swap, Ordering::Acquire because predicate may need
    // to see changes in the objects themselves
    let mut iter_current_ptr = self.head.swap(ptr::null_mut(), Ordering::Acquire);
    
    // Run 'predicate' for each object, so can decide whether to deallocate or not
    while iter_current_ptr != ptr::null_mut() {
      let current_ptr = iter_current_ptr;
      
      // Note: Ordering::Acquire because changes in 'next' object need to be visible
      // SAFETY: 'current' is valid because is leaked and only deallocated by current
      // thread and the list only ever appended outside of this method
      let current = unsafe { &mut *current_ptr };
      iter_current_ptr = current.next.load(Ordering::Acquire);
      
      let current_ptr_as_usize = current_ptr as usize;
      if !predicate(current) {
        println!("Dead        : {current_ptr_as_usize:#016x}");
        // 'predicate' determine that 'current' object is to be deallocated
        self.dealloc(current);
        
        // 'current' reference now is invalid, drop it to let compiler
        // know that it can't be used anymore and as a note to future
        // me UwU
        #[allow(dropping_references)]
        drop(current);
        continue;
      }
      println!("Live        : {current_ptr_as_usize:#016x}");
      
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
      self.add_chain_to_list(live_objects, last_live_objects);
    }
  }
}


