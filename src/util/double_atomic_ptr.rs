use std::{marker::PhantomData, sync::atomic::Ordering};

#[cfg(target_pointer_width = "64")]
use portable_atomic::AtomicU128;

#[cfg(target_pointer_width = "32")]
use portable_atomic::AtomicU64;
use static_assertions::const_assert_eq;

// Like AtomicPtr<T> but atomically exchange
// two pointers
pub struct AtomicDoublePtr<T1, T2> {
  double_ptr: AtomicDPointers,
  phantom: PhantomData<(T1, T2)>
}

#[cfg(target_pointer_width = "64")]
type DPointers = u128;
#[cfg(target_pointer_width = "32")]
type DPointers = u64;

#[cfg(target_pointer_width = "64")]
type AtomicDPointers = AtomicU128;
#[cfg(target_pointer_width = "32")]
type AtomicDPointers = AtomicU64;

// Double pointer indeed must fit two pointers
// as usize must be single pointer sized
const_assert_eq!(DPointers::BITS, usize::BITS * 2);

impl<T1, T2> AtomicDoublePtr<T1, T2> {
  pub fn new(ptrs: (*mut T1, *mut T2)) -> Self {
    return Self {
      double_ptr: AtomicDPointers::new(Self::pack_pointers(ptrs)),
      phantom: PhantomData {}
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
  
  pub fn store(&self, ptrs: (*mut T1, *mut T2), ordering: Ordering) {
    self.double_ptr.store(Self::pack_pointers(ptrs), ordering);
  }
  
  pub fn load(&self, ptrs: (*mut T1, *mut T2), ordering: Ordering) -> (*mut T1, *mut T2) {
    return Self::unpack_pointers(self.double_ptr.load(ordering));
  }
  
  pub fn swap(&self, ptrs: (*mut T1, *mut T2), ordering: Ordering) -> (*mut T1, *mut T2) {
    return Self::unpack_pointers(self.double_ptr.swap(Self::pack_pointers(ptrs), ordering));
  }
}

