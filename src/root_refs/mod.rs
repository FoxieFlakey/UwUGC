// A abstraction layer which can specialize RootRef
// into various level of exclusiveness

use std::ops::{Deref, DerefMut};

use crate::objects_manager::ObjectLikeTrait;
use crate::heap::RootRefRaw;

// This is like &mut but the reference is locked on
// current thread (due !Sync + !Send from RootRefRaw)
pub struct RootRefExclusive<'a, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, T>
}

impl<'a, T: ObjectLikeTrait> RootRefExclusive<'a, T> {
  // SAFETY: The caller must ensure that the reference is exclusive
  pub(crate) unsafe fn new(inner: RootRefRaw<'a, T>) -> Self {
    return Self {
      inner
    }
  }
  
  pub fn downgrade(this: RootRefExclusive<'a, T>) -> RootRefShared<'a, T> {
    // SAFETY: This is safe because Rust borrowing rules prevents escape of
    // mutable reference which makes shared reference bad
    return unsafe { RootRefShared::new(this.inner) };
  }
}

impl<'a, T: ObjectLikeTrait> Deref for RootRefExclusive<'a, T> {
  type Target = T;
  
  fn deref(&self) -> &Self::Target {
    // SAFETY: This is an exclusive reference, no problem exists
    return unsafe { self.inner.borrow_inner() };
  }
}

impl<'a, T: ObjectLikeTrait> DerefMut for RootRefExclusive<'a, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    // SAFETY: This is an exclusive reference, no problem exists
    return unsafe { self.inner.borrow_inner_mut() };
  }
}

// This is like & but the reference is locked on
// current thread (due !Sync + !Send from RootRefRaw)
pub struct RootRefShared<'a, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, T>
}

impl<'a, T: ObjectLikeTrait> RootRefShared<'a, T> {
  // SAFETY: The caller must ensure that the reference can be
  // safely shared and is is another name for immutable reference
  pub(crate) unsafe fn new(inner: RootRefRaw<'a, T>) -> Self {
    return Self {
      inner
    }
  }
  
  pub fn into_raw(this: RootRefShared<'a, T>) -> RootRefRaw<'a, T> {
    return this.inner;
  }
}

impl<'a, T: ObjectLikeTrait> Deref for RootRefShared<'a, T> {
  type Target = T;
  
  fn deref(&self) -> &Self::Target {
    // SAFETY: This is an shared reference without ability to get mutable
    // reference to it
    return unsafe { self.inner.borrow_inner() };
  }
}

