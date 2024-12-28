use std::{alloc::Layout, any::TypeId, cell::UnsafeCell, marker::PhantomData, ptr::NonNull, sync::{atomic::{self, Ordering}, Arc}, thread};

use crate::{allocator::HeapAlloc, descriptor::SELF_DESCRIPTOR, ReferenceType};

use crate::{descriptor::{DescriptorInternal, Describeable}, gc::GCLockCookie, objects_manager::{Object, ObjectLikeTraitInternal}};

use super::{AllocError, ObjectManager};

pub struct LocalObjectsChain {
  // Maintains start and end of chain
  start: UnsafeCell<Option<*mut Object>>,
  end: UnsafeCell<Option<*mut Object>>
}

// SAFETY: Accesses to this is protected by GC lock and lock on 'contexts'
// in ObjectManager structure
unsafe impl Sync for LocalObjectsChain {}
unsafe impl Send for LocalObjectsChain {}

impl LocalObjectsChain {
  pub fn new() -> Self {
    Self {
      start: UnsafeCell::new(None),
      end: UnsafeCell::new(None)
    }
  }
  
  // Move all objects in local chain to global list
  // clearing local chain
  // SAFETY: Caller must ensure that 'owner' is actually the owner
  // and make sure that this chain isn't concurrently accessed
  // Concurrent access can be protected either by
  // 1. preventing Sweeper (the only other thing which concurrently access)
  //     from getting exclusive GC lock
  // 2. locks the 'contexts' as Sweeper also needs it
  pub unsafe fn flush_to_global<A: HeapAlloc>(&self, owner: &ObjectManager<A>) {
    // Make sure newest object added by mutator visible to current thread
    // (which might be other thread than the mutator)
    atomic::fence(Ordering::Acquire);
    
    // SAFETY: Caller make sure that LocalObjectsChain not concurrently accessed
    let (start, end) = unsafe { (&mut *self.start.get(), &mut *self.end.get()) };
    
    // Nothing to flush
    if start.is_none() {
      assert!(start.is_none());
      assert!(end.is_none());
      return;
    }
    
    // SAFETY: All objects in chain are valid, by design random object in middle
    // of chain cannot be deallocated safely
    unsafe {
      owner.add_chain_to_list(start.unwrap(), end.unwrap());
    }
    
    // Clear the list
    *start = None;
    *end = None;
    
    // Make the changes visible to the mutator so it can properly start new chain
    atomic::fence(Ordering::Release);
  }
}

pub struct Handle<'a, A: HeapAlloc> {
  ctx: Arc<LocalObjectsChain>,
  owner: &'a ObjectManager<A>,
  // Ensure that ContextHandle stays on same thread
  // by disallowing it to be Send or Sync
  _phantom: PhantomData<*const ()>
}

impl<'a, A: HeapAlloc> Handle<'a, A> {
  pub(super) fn new(ctx: Arc<LocalObjectsChain>, owner: &'a ObjectManager<A>) -> Self {
    Self {
      owner,
      ctx,
      _phantom: PhantomData {}
    }
  }
  
  // SAFETY: Caller has to ensure layout is correct for the data contained
  // so 'usage' can be counted correctly
  unsafe fn try_alloc_unchecked<F: FnOnce() -> Result<NonNull<Object>, AllocError>>(&self, func: F, data_layout: Layout, _gc_lock_cookie: &mut GCLockCookie<A>) -> Result<*mut Object, AllocError> {
    let manager = self.owner;
    let object_size = Object::calc_layout(&data_layout).0.size();
    manager.used_size.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |mut x| {
      x += object_size;
      
      if x <= self.owner.max_size {
        Some(x)
      } else {
        None
      }
    }).map_err(|_| AllocError)?;
    
    // Allocate the object
    let Ok(obj) = func() else {
        // Undo what just did to the usage counter
        self.owner.used_size.fetch_sub(object_size, Ordering::Relaxed);
        return Err(AllocError);
      };
    let obj_ptr = obj.as_ptr();
    
    // SAFETY: Just allocated it before
    let obj_header = unsafe { obj_ptr.as_ref().unwrap_unchecked() };
    
    // Ensure changes made previously by potential flush_to_global
    // emptying the local list visible to this
    atomic::fence(Ordering::Acquire);
    
    // Add object to local chain
    // SAFETY: Safe because the concurrent access by other is protected by GC lock
    // See comment for LocalObjectsChain#flush_to_global method
    let start = unsafe { self.ctx.start.get().as_mut().unwrap_unchecked() };
    let end = unsafe { self.ctx.end.get().as_mut().unwrap_unchecked() };
    match start.as_mut() {
      // The list has some objects, append current 'start' to end of this object
      // SAFETY: The object isn't visible yet to other thread so its safe from
      // concurrent accesses
      Some(x) => unsafe { *obj_header.next.get() = *x },
      
      // The list was empty this object is the 'end' of current list
      None => *end = Some(obj_ptr)
    }
    
    // Update the 'start' so it point to newly made object
    *start = Some(obj_ptr);
    
    // Make sure potential flush_to_global can see latest items
    atomic::fence(Ordering::Release);
    Ok(obj_ptr)
  }
  
  // pub fn try_alloc<T: Describeable + ObjectLikeTraitInternal, F: FnMut() -> T>(&self, func: F, gc_lock_cookie: &mut GCLockCookie<A>) -> Result<*mut Object, AllocError> {
  //   // SAFETY: The descriptor is correct and retrieved from T::get_descriptor so the
  //   // correctnes of it depends on the unsafe trait Describeable being upheld by the
  //   // type
  //   unsafe { self.try_alloc_common(func, gc_lock_cookie, DescriptorInternal { api: T::get_descriptor(), drop_helper: T::drop_helper }) }
  // }
  
  pub fn try_alloc_array<Ref: ReferenceType, F: FnMut() -> [Ref; LEN], const LEN: usize>(&self, func: F, gc_lock_cookie: &mut GCLockCookie<A>) -> Result<*mut Object, AllocError> {
    let (array_layout, stride) = Layout::new::<Ref>()
      .repeat(LEN)
      .unwrap();
    
    // Not sure if above is correct because Rust's [T; N] array
    // is array where 'stride' of it is the size of T itself
    assert!(stride == size_of::<Ref>(), "Strange corner case?");
    
    // SAEFTY: Layout is correct for given type
    unsafe { self.try_alloc_unchecked(|| Object::new_array(self.owner, func), array_layout, gc_lock_cookie) }
  }
  
  pub fn try_alloc<T: Describeable + ObjectLikeTraitInternal, F: FnMut() -> T>(&self, func: F, gc_lock_cookie: &mut GCLockCookie<A>) -> Result<*mut Object, AllocError> {
    let mut desc_cache = self.owner.descriptor_cache.upgradable_read();
    let id = TypeId::of::<T>();
    
    let descriptor_obj_ptr ;
    let mut from_cache ;
    if let Some(&x) = desc_cache.get(&id) {
      // The descriptor is cached, lets get pointer to it
      from_cache = true;
      descriptor_obj_ptr = x;
    } else {
      // The descriptor isnt cached, create new one
      from_cache = false;
      descriptor_obj_ptr = desc_cache.with_upgraded(|desc_cache| {
        if let Some(&x) = desc_cache.get(&id) {
          // Another thread has already insert it to cache, no need to create
          // it again
          from_cache = true;
          return Ok(x);
        }
        
        // Directly call unchecked alloc, because to avoid resulting in
        // chicken and egg problem because to allocate descriptor in heap
        // there has to be already existing descriptor in heap so break
        // the cycle with statically allocated 'root' descriptor.
        //
        // SAFETY: The descriptor is correct for Descriptor because it None
        // for object pointer which is treated as special value to reference
        // statically declared descriptor::SELF_DESCRIPTOR
        let new_descriptor = unsafe {
          NonNull::new_unchecked(
            self.try_alloc_unchecked(|| {
                Object::new(self.owner, || DescriptorInternal {
                  api: T::get_descriptor(),
                  drop_helper: T::drop_helper
                }, None)
              },
              SELF_DESCRIPTOR.layout,
              gc_lock_cookie
            )?
          )
        };
        
        // If not present in cache, try insert into it with upgraded rwlock
        Ok(
          *desc_cache.entry(id)
            .or_insert(new_descriptor)
        )
      })?;
    }
    
    // SAFETY: It can't be GC'ed away because GC is being blocked
    // so it is valid
    let obj_ref = unsafe { descriptor_obj_ptr.as_ref() };
    
    if from_cache {
      // Activate GC's load barrier because it wanted to know that descriptor still
      // in use if its fetched from the cache
      gc_lock_cookie.get_gc().load_barrier(obj_ref, self.owner, gc_lock_cookie);
    }
    
    // SAFETY: descriptor still alive because GC is blocked so it can
    // be gone
    let data_layout = unsafe {
        Object::get_raw_ptr_to_data(descriptor_obj_ptr)
          .cast::<DescriptorInternal>()
          .as_ref()
          .layout
      };
    
    // SAFETY: Already make sure that the descriptor is correct
    unsafe {
      self.try_alloc_unchecked(|| {
          Object::new(
            self.owner,
            func,
            Some(descriptor_obj_ptr)
          )
        },
        data_layout,
        gc_lock_cookie
      )
    }
  }
}

impl<A: HeapAlloc> Drop for Handle<'_, A> {
  fn drop(&mut self) {
    let mut contexts = self.owner.contexts.lock();
    
    // Move all objects in current local chain to global list
    // SAFETY: Concurrent access can't happen because of Sweeper
    // needs 'contexts' to be locked
    unsafe { self.ctx.flush_to_global(self.owner); };
    
    // Remove context belonging to current thread
    contexts.remove(&thread::current().id());
  }
}

