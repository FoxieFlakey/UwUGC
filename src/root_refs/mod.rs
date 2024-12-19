// A abstraction layer which can specialize RootRef
// into various level of exclusiveness

use std::ops::{Deref, DerefMut};

use sealed::sealed;

use crate::objects_manager::ObjectLikeTrait;
use crate::heap::context::RootRefRaw;

// Sealed because there no need
// for other things to implement more kinds as there only
// two kinds which are Shared (or immutable) and Exclusive (or mutable)
// both can be sent to other thread
#[sealed]
pub trait RefKind: Default {}

#[derive(Default)]
pub struct Exclusive {}
#[sealed]
impl RefKind for Exclusive {}

#[derive(Default)]
pub struct Shared {}
#[sealed]
impl RefKind for Shared {}

pub type RootRefExclusive<'a, T> = RootRef<'a, Exclusive, T>;
pub type RootRefShared<'a, T> = RootRef<'a, Shared, T>;

// This is like &mut but the reference is locked on
// current thread (due !Sync + !Send from RootRefRaw)
pub struct RootRef<'a, Kind: RefKind, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, T>,
  _kind: Kind
}

impl<'a, Kind: RefKind, T: ObjectLikeTrait> RootRef<'a, Kind, T> {
  pub unsafe fn new(inner: RootRefRaw<'a, T>) -> Self {
    return Self {
      inner,
      _kind: Kind::default()
    }
  }
  
  pub fn into_raw(this: Self) -> RootRefRaw<'a, T> {
    return this.inner;
  }
}

impl<'a, Kind: RefKind, T: ObjectLikeTrait> Deref for RootRef<'a, Kind, T> {
  type Target = T;
  
  fn deref(&self) -> &Self::Target {
    // SAFETY: All root refs can be immutably borrow
    // and safety of not having another mutable reference
    // is ensure by API design of one way downgrade to shared
    // before it is sent to other thread
    return unsafe { self.inner.borrow_inner() };
  }
}

impl<'a, T: ObjectLikeTrait> RootRef<'a, Exclusive, T> {
  pub fn downgrade(this: Self) -> RootRef<'a, Shared, T> {
    // SAFETY: This is exclusive borrow and it is safe to downgrade
    // to shared
    return unsafe { RootRef::new(this.inner) };
  }
}

impl<'a, T: ObjectLikeTrait> DerefMut for RootRef<'a, Exclusive, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    // SAFETY: Only exclusive root ref can be mutably
    // borrowed and API design ensure there no other
    // immutable borrows
    return unsafe { self.inner.borrow_inner_mut() };
  }
}

