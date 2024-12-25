use std::{cell::UnsafeCell, marker::{PhantomData, PhantomPinned}, pin::Pin, ptr::{self, NonNull}, sync::{atomic, Arc}, thread};

use crate::allocator::HeapAlloc;

use super::{Heap, RootEntry};
use crate::{descriptor::Describeable, gc::GCLockCookie, objects_manager::{self, Object}, root_refs::{Exclusive, RootRef, Sendable}, ObjectLikeTraitInternal};

pub struct Data<A: HeapAlloc> {
  head: Pin<Box<RootEntry<A>>>
}

pub struct DataWrapper<A: HeapAlloc> {
  inner: UnsafeCell<Data<A>>
}

// SAFETY: Manually enforces safety of concurrently accessing it
// by GC lock, and GC is only the other thread which reads this
// while the owning thread is the only writer
unsafe impl<A: HeapAlloc> Sync for DataWrapper<A> {}

// This type exists so that any API can enforce that
// it is being constructed/called inside a special context which
// only be made available by the 'alloc' function as creating
// GC refs anywhere else is always unsafe due the assumptions
// it needs
pub struct ConstructorScope {
  _private: ()
}

impl<A: HeapAlloc> DataWrapper<A> {
  pub fn new() -> Self {
    let head = Box::pin(RootEntry {
      gc_state: ptr::null_mut(),
      obj: ptr::null_mut(),
      next: UnsafeCell::new(ptr::null()),
      prev: UnsafeCell::new(ptr::null()),
      
      _phantom: PhantomPinned
    });
    
    // SAFETY: There no way concurrent access can happen yet
    // and root set need to be circular list
    unsafe {
      *head.next.get() = &*head;
      *head.prev.get() = &*head;
    }
    
    Self {
      inner: UnsafeCell::new(Data {
        head
      })
    }
  }
  
  // SAFETY: Caller ensures that thread managing the context does not
  // concurrently runs with this function (usually meant mutator
  // threads is being blocked)
  pub(super) unsafe fn for_each_root(&self, mut iterator: impl FnMut(&RootEntry<A>)) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    // SAFETY: Caller ensured mutators are blocked so nothing modifies this
    let inner = unsafe { &*self.inner.get() };
    let head = inner.head.as_ref().get_ref();
    
    // SAFETY: In circular buffer 'next' is always valid
    let mut current = unsafe { &*(*head.next.get()) };
    // While 'current' is not the head as this linked list is circular
    while ptr::from_ref(current) != ptr::from_ref(head) {
      iterator(current);
      // SAFETY: In circular buffer 'next' is always valid
      current = unsafe { &*(*current.next.get()) };
    }
  }
  
  // SAFETY: Caller ensures that nothing can concurrently access the 
  // root set
  pub unsafe fn clear_root_set(&self) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    
    // SAFETY: Caller ensured mutators are blocked so nothing modifies this
    let inner = unsafe { &*self.inner.get() };
    let head = inner.head.as_ref().get_ref();
    
    // SAFETY: Caller ensured that nothing accessed the root set concurrently
    let mut current = unsafe { *head.next.get() };
    // While 'current' is not the head as this linked list is circular
    while current != ptr::from_ref(head) {
      // SAFETY: Guaranteed by caller that root set is not accessed concurrently
      let next = unsafe { *(*current).next.get() };
      
      // Drop the root entry and remove it from set
      // SAFETY: Guaranteed by caller that root set is not accessed concurrently
      let _ = unsafe { Box::from_raw(current.cast_mut()) };
      current = next;
    }
  }
}

impl<A: HeapAlloc> Drop for DataWrapper<A> {
  fn drop(&mut self) {
    // Current thread is last one with reference to this context
    // therefore its safe to clear it (to deallocate the root entries)
    unsafe { self.clear_root_set() };
  }
}

pub struct Context<'a, A: HeapAlloc> {
  ctx: Arc<DataWrapper<A>>,
  obj_manager_ctx: objects_manager::Handle<'a, A>,
  owner: &'a Heap<A>,
  // ContextHandle will only stays at current thread
  _phantom: PhantomData<*const ()>
}

pub struct RootRefRaw<'a, A: HeapAlloc, T: ObjectLikeTraitInternal> {
  entry_ref: *const RootEntry<A>,
  _phantom: PhantomData<&'a T>,
  // RootRef will only stays at current thread
  _force_not_send_sync: PhantomData<*const ()>
}

impl<A: HeapAlloc, T: ObjectLikeTraitInternal> RootRefRaw<'_, A, T> {
  fn get_raw_ptr_to_data(&self) -> NonNull<()> {
    // SAFETY: References are always non null
    let obj = unsafe { NonNull::new_unchecked(ptr::from_ref(self.get_object_borrow()).cast_mut()) };
    
    // SAFETY: As long as RootRefRaw exist object pointer will remains valid
    unsafe { Object::get_raw_ptr_to_data(obj) }
  }
  
  // SAFETY: The root reference may not be safe in face of
  // data race, it is up to caller to ensure its safe
  pub unsafe fn borrow_inner(&self) -> &T {
    // SAFETY: Type already statically checked by Rust
    // via this type's T and caller ensure safetyness
    // of making the reference
    unsafe { self.get_raw_ptr_to_data().cast::<T>().as_ref() }
  }
  
  // SAFETY: The root reference may not be safe in face of
  // data race, it is up to caller to ensure its safe
  pub unsafe fn borrow_inner_mut(&mut self) -> &mut T {
    // SAFETY: Type already statically checked by Rust
    // via this type's T and caller ensure safetyness
    // of making the reference
    unsafe { self.get_raw_ptr_to_data().cast::<T>().as_mut() }
  }
  
  pub fn get_object_borrow(&self) -> &Object {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    let root_entry = unsafe { &*self.entry_ref };
    // SAFETY: Objects accessible by this root reference guaranteed
    // to be alive by the GC
    unsafe { &*root_entry.obj }
  }
}

impl<A: HeapAlloc, T: ObjectLikeTraitInternal> Drop for RootRefRaw<'_, A, T> {
  fn drop(&mut self) {
    // Corresponding RootEntry and RootRef are free'd together
    // therefore its safe after removing reference from root set
    // SAFETY: The reference to the entry is managed by the same
    // thread which created it
    let entry = unsafe { &*self.entry_ref };
    
    // Block GC as GC would see half modified root set if without
    // SAFETY: GCState is always valid
    let cookie = unsafe { &*entry.gc_state }.block_gc();
    
    // SAFETY: Circular linked list is special that every next and prev
    // is valid so its safe and GC is blocked so GC does not attempting
    // to access root set
    let next_ref = unsafe { &*(*entry.next.get()) };
    let prev_ref = unsafe { &*(*entry.prev.get()) };
    
    // Actually removes
    // SAFETY: GC is blocked so GC does not attempting
    // to access root set
    unsafe {
      *next_ref.prev.get() = prev_ref;
      *prev_ref.next.get() = next_ref;
    };
    
    // Let GC run again and Release fence to allow GC to see
    // removal of current entry (Acquire not needed as there
    // no other writer thread other than GC which only ever
    // does read)
    atomic::fence(atomic::Ordering::Release);
    drop(cookie);
    
    // Make sure that 'entry' reference will never be invalid
    // by telling Rust its lifetime ends here
    #[allow(dropping_references)]
    drop(entry);
    
    // Drop the "root_entry" itself as its unused now
    // SAFETY: Nothing reference it anymore so it is safe
    // to be dropped and casted to *mut pointer
    let _ = unsafe { Box::from_raw(self.entry_ref.cast_mut()) };
  }
}

impl<'a, A: HeapAlloc> Context<'a, A> {
  pub(super) fn new(owner: &'a Heap<A>, obj_manager_ctx: objects_manager::Handle<'a, A>, ctx: Arc<DataWrapper<A>>) -> Self {
    Self {
      ctx,
      owner,
      obj_manager_ctx,
      _phantom: PhantomData {}
    }
  }
  
  pub fn get_heap(&self) -> &Heap<A> {
    self.owner
  }
  
  pub fn new_root_ref_from_ptr<T: ObjectLikeTraitInternal>(&self, ptr: *mut Object, _gc_lock_cookie: &mut GCLockCookie<A>) -> RootRefRaw<'a, A, T> {
    let entry = Box::new(RootEntry {
      gc_state: &self.owner.gc,
      obj: ptr,
      next: UnsafeCell::new(ptr::null()),
      prev: UnsafeCell::new(ptr::null()),
      
      _phantom: PhantomPinned
    });
    
    // SAFETY: Current thread is only owner of the head, and modification to it
    // is protected by GC locks by requirement of '_gc_lock_cookie' mutable reference
    // which requires that GC is blocked to have a reference to it
    //
    // therefore, current thread modifies it and GC won't be able to concurrently
    // access it
    let entry = unsafe { (*self.ctx.inner.get()).head.insert(entry) };
    
    // Release fence to allow newly added value to be
    // visible to the GC
    atomic::fence(atomic::Ordering::Release);
    RootRefRaw {
      entry_ref: entry,
      _phantom: PhantomData {},
      _force_not_send_sync: PhantomData {}
    }
  }
  
  pub fn alloc<T: Describeable + ObjectLikeTraitInternal>(&mut self, initer: impl FnOnce(&mut ConstructorScope) -> T) -> RootRef<'a, Sendable, Exclusive, A, T> {
    // Shouldn't panic if try_alloc succeded once, and with this
    // method this function shouldnt try alloc again
    let mut special_ctx = ConstructorScope { _private: () };
    let mut inited_value = Some(initer);
    let mut must_init_once = || inited_value.take().unwrap()(&mut special_ctx);
    
    let mut gc_lock_cookie = self.owner.gc.block_gc();
    let mut obj = self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie);
    
    if obj.is_err() {
      drop(gc_lock_cookie);
      println!("Out of memory, triggering GC!");
      self.owner.run_gc();
      gc_lock_cookie = self.owner.gc.block_gc();
      
      obj = self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie);
      assert!(obj.is_ok(), "Heap run out of memory!");
    }
    
    let root_ref = self.new_root_ref_from_ptr(obj.unwrap(), &mut gc_lock_cookie);
    // SAFETY: The object reference is exclusively owned by this thread
    unsafe { RootRef::new(root_ref) }
  }
}

impl<A: HeapAlloc> Drop for Context<'_, A> {
  fn drop(&mut self) {
    // Remove context belonging to current thread
    self.owner.contexts.lock().remove(&thread::current().id());
  }
}

