use std::{cell::UnsafeCell, marker::PhantomData, mem::MaybeUninit, ptr::NonNull, sync::{atomic, Arc}, thread};

use crate::{allocator::HeapAlloc, gc::GCState, ReferenceType};

use super::{root_set::RootSet, Heap, RootEntry};
use crate::{descriptor::Describeable, gc::GCLockCookie, objects_manager::{self, Object}, ObjectLikeTraitInternal};

pub struct ContextData<A: HeapAlloc> {
  inner: UnsafeCell<RootSet<A>>
}

// SAFETY: Manually enforces safety of concurrently accessing it
// by GC lock, and GC is only the other thread which reads this
// while the owning thread is the only writer
unsafe impl<A: HeapAlloc> Sync for ContextData<A> {}

// This type exists so that any API can enforce that
// it is being constructed/called inside a special context which
// only be made available by the 'alloc' function as creating
// GC refs anywhere else is always unsafe due the assumptions
// it needs
pub struct ConstructorScope {
  _private: ()
}

impl<A: HeapAlloc> ContextData<A> {
  // SAFETY: Caller ensure that gc_state lives longer than
  // this data wrapper
  pub unsafe fn new(gc_state: NonNull<GCState<A>>) -> Self {
    Self {
      // SAFETY: Caller already ensured that gc_state lives longer than
      // this data wrapper
      inner: UnsafeCell::new(unsafe { RootSet::new(gc_state) })
    }
  }
  
  // SAFETY: Caller ensures that thread managing the context is not actively
  // accessing this context
  pub(super) unsafe fn take_root_set_snapshot(&self, buffer: &mut Vec<NonNull<Object>>) {
    // Make sure any newly added/removed root entry is visible
    atomic::fence(atomic::Ordering::Acquire);
    // SAFETY: Caller ensured mutators are blocked so nothing modifies this
    let inner = unsafe { &*self.inner.get() };
    inner.take_snapshot(buffer);
  }
}

pub struct Context<'a, A: HeapAlloc> {
  ctx: Arc<ContextData<A>>,
  obj_manager_ctx: objects_manager::Handle<'a, A>,
  owner: &'a Heap<A>,
  // ContextHandle will only stays at current thread
  _phantom: PhantomData<*const ()>
}

pub struct RootRefRaw<'a, A: HeapAlloc, T: ObjectLikeTraitInternal> {
  entry_ref: NonNull<RootEntry<A>>,
  _phantom: PhantomData<&'a T>,
  // RootRef will only stays at current thread
  _force_not_send_sync: PhantomData<*const ()>
}

impl<A: HeapAlloc, T: ObjectLikeTraitInternal> RootRefRaw<'_, A, T> {
  fn get_raw_ptr_to_data(&self) -> NonNull<()> {
    // SAFETY: As long as RootRefRaw exist object pointer will remains valid
    unsafe { Object::get_raw_ptr_to_data(self.get_object_ptr()) }
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
  
  pub fn get_object_ptr(&self) -> NonNull<Object> {
    // SAFETY: root_entry is managed by current thread
    // so it can only be allocated and deallocated on
    // same thread
    unsafe { self.entry_ref.as_ref() }.get_obj_ptr()
  }
}

impl<A: HeapAlloc, T: ObjectLikeTraitInternal> Drop for RootRefRaw<'_, A, T> {
  fn drop(&mut self) {
    // Block GC as GC would see half modified root set if without it
    // SAFETY: GCState is always valid
    let cookie = unsafe { self.entry_ref.as_ref().get_gc_state().as_ref() }.block_gc();
    
    // Corresponding RootEntry and RootRef are free'd together
    // therefore its safe after removing reference from root set
    // SAFETY: The reference to the entry is managed by the same
    // thread which created it
    unsafe { RootEntry::delete(self.entry_ref) };
    
    // Release fence to make deletion visible to GC
    atomic::fence(atomic::Ordering::Release);
    drop(cookie);
  }
}

impl<'a, A: HeapAlloc> Context<'a, A> {
  pub(super) fn new(owner: &'a Heap<A>, obj_manager_ctx: objects_manager::Handle<'a, A>, ctx: Arc<ContextData<A>>) -> Self {
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
  
  pub fn new_root_ref_from_ptr<T: ObjectLikeTraitInternal>(&self, ptr: NonNull<Object>, _gc_lock_cookie: &mut GCLockCookie<A>) -> RootRefRaw<'a, A, T> {
    // SAFETY: GC is blocked due existence of lock cookie and GC is only other
    // thing which access the root
    let entry = unsafe { (*self.ctx.inner.get()).insert(ptr) };
    
    // Release fence to allow newly added value to be
    // visible to the GC
    // NOTE: There is no acquire fence because
    // GC won't be modifying the list so that is not needed
    atomic::fence(atomic::Ordering::Release);
    RootRefRaw {
      entry_ref: entry,
      _phantom: PhantomData {},
      _force_not_send_sync: PhantomData {}
    }
  }
  
  // SAFETY: Caller must make sure that initializer properly initialize T
  pub unsafe fn alloc<T: Describeable + ObjectLikeTraitInternal>(&self, initer: impl FnOnce(&mut ConstructorScope, &mut MaybeUninit<T>)) -> RootRefRaw<'a, A, T> {
    // Shouldn't panic if try_alloc succeded once, and with this
    // method this function shouldnt try alloc again
    let mut special_ctx = ConstructorScope { _private: () };
    let mut inited_value = Some(initer);
    let mut must_init_once = |uninit: &mut MaybeUninit<T>| inited_value.take().unwrap()(&mut special_ctx, uninit);
    
    let mut gc_lock_cookie = self.owner.gc.block_gc();
    // SAFETY: Caller already make sure that initializer properly initialize T
    let mut obj = unsafe { self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie) };
    
    if obj.is_err() {
      drop(gc_lock_cookie);
      println!("Out of memory, triggering GC!");
      self.owner.run_gc(true);
      gc_lock_cookie = self.owner.gc.block_gc();
      
      
      // SAFETY: Caller already make sure that initializer properly initialize T
      obj = unsafe { self.obj_manager_ctx.try_alloc(&mut must_init_once, &mut gc_lock_cookie) };
      assert!(obj.is_ok(), "Heap run out of memory!");
    }
    
    let root_ref = self.new_root_ref_from_ptr(obj.unwrap(), &mut gc_lock_cookie);
    root_ref
  }
  
  // TODO: Try deduplicate alloc and alloc_array without coming a foul with borrow
  // checker
  // SAFETY: Initializer has to make sure that array is properly initialized
  pub unsafe fn alloc_array<Ref: ReferenceType, const LEN: usize>(&self, initer: impl FnOnce(&mut ConstructorScope, &mut MaybeUninit<[Ref; LEN]>)) -> RootRefRaw<'a, A, [Ref; LEN]> {
    // Shouldn't panic if try_alloc succeded once, and with this
    // method this function shouldnt try alloc again
    let mut special_ctx = ConstructorScope { _private: () };
    let mut inited_value = Some(initer);
    let mut must_init_once = |uninit: &mut MaybeUninit<[Ref; LEN]>| inited_value.take().unwrap()(&mut special_ctx, uninit);
    
    let mut gc_lock_cookie = self.owner.gc.block_gc();
    
    // SAFETY: Caller already make sure that initializer properly initialize the array
    let mut obj = unsafe { self.obj_manager_ctx.try_alloc_array(&mut must_init_once, &mut gc_lock_cookie) };
    
    if obj.is_err() {
      drop(gc_lock_cookie);
      println!("Out of memory, triggering GC!");
      self.owner.run_gc(true);
      gc_lock_cookie = self.owner.gc.block_gc();
      
      // SAFETY: Caller already make sure that initializer properly initialize the array
      obj = unsafe { self.obj_manager_ctx.try_alloc_array(&mut must_init_once, &mut gc_lock_cookie) };
      assert!(obj.is_ok(), "Heap run out of memory!");
    }
    
    let root_ref = self.new_root_ref_from_ptr(obj.unwrap(), &mut gc_lock_cookie);
    root_ref
  }
}

impl<A: HeapAlloc> Drop for Context<'_, A> {
  fn drop(&mut self) {
    // Remove context belonging to current thread
    self.owner.contexts.lock().remove(&thread::current().id());
  }
}

