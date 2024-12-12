use std::{marker::PhantomData, sync::atomic::Ordering};

#[cfg(target_pointer_width = "64")]
use portable_atomic::AtomicU128;

#[cfg(target_pointer_width = "32")]
use portable_atomic::AtomicU64;

// Like AtomicPtr<T> but atomically exchange
// two pointers
pub struct AtomicDoublePtr<T1, T2> {
  double_ptr: AtomicDPointers,
  _phantom: PhantomData<(T1, T2)>
}

#[cfg(target_pointer_width = "64")]
type DPointers = u128;
#[cfg(target_pointer_width = "32")]
type DPointers = u64;

#[cfg(target_pointer_width = "64")]
type AtomicDPointers = AtomicU128;
#[cfg(target_pointer_width = "32")]
type AtomicDPointers = AtomicU64;

impl<T1, T2> AtomicDoublePtr<T1, T2> {
  pub fn new(ptrs: (*mut T1, *mut T2)) -> Self {
    return Self {
      double_ptr: AtomicDPointers::new(Self::pack_pointers(ptrs)),
      _phantom: PhantomData {}
    };
  }
  
  fn pack_pointers(ptrs: (*mut T1, *mut T2)) -> DPointers {
    let a = ptrs.0 as DPointers;
    let b = ptrs.1 as DPointers;
    return a | (b << usize::BITS);
  }
  
  fn unpack_pointers(dptr: DPointers) -> (*mut T1, *mut T2) {
    let a = (dptr & (usize::MAX as DPointers - 1)) as usize;
    let b = ((dptr >> usize::BITS) & (usize::MAX as DPointers - 1)) as usize;
    return (a as *mut T1, b as *mut T2);
  }
  
  pub fn load(&self, ordering: Ordering) -> (*mut T1, *mut T2) {
    return Self::unpack_pointers(self.double_ptr.load(ordering));
  }
  
  pub fn swap(&self, ptrs: (*mut T1, *mut T2), ordering: Ordering) -> (*mut T1, *mut T2) {
    return Self::unpack_pointers(self.double_ptr.swap(Self::pack_pointers(ptrs), ordering));
  }
  
  pub fn compare_exchange_weak(&self, old: (*mut T1, *mut T2), new: (*mut T1, *mut T2), success: Ordering, failure: Ordering) -> Result<(*mut T1, *mut T2), (*mut T1, *mut T2)> {
    let old = Self::pack_pointers(old);
    let new = Self::pack_pointers(new);
    
    return match self.double_ptr.compare_exchange_weak(old, new, success, failure) {
      Ok(x) => Ok(Self::unpack_pointers(x)),
      Err(x) => Err(Self::unpack_pointers(x))
    };
  }
}

