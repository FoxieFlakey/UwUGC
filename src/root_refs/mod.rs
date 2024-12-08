// A abstraction layer which can specialize RootRef
// into various level of exclusiveness

use std::ops::{Deref, DerefMut};

use crate::objects_manager::ObjectLikeTrait;
use crate::heap::RootRefRaw;

pub struct RootRefExclusive<'a, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, T>
}

impl<'a, T: ObjectLikeTrait> RootRefExclusive<'a, T> {
  // SAFETY: The caller has to ensure that the given root
  // reference is safe to use exclusively and ensure there
  // no other exclusive reference to it exists
  //
  // Acts like Rust's Box<T>
  pub(crate) unsafe fn new(inner: RootRefRaw<'a, T>) -> Self {
    return Self {
      inner
    }
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

