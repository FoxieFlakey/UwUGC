// A abstraction layer which can specialize RootRef
// into various level of exclusiveness

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use sealed::sealed;

use crate::objects_manager::ObjectLikeTrait;
use crate::heap::context::RootRefRaw;

// NOTE: This type is considered to be part of public API
#[sealed]
pub trait RefKind {}

// NOTE: This type is considered to be part of public API
pub struct Exclusive {}
#[sealed]
impl RefKind for Exclusive {}

// NOTE: This type is considered to be part of public API
pub struct Shared {}
#[sealed]
impl RefKind for Shared {}

// NOTE: This type is considered to be part of public API
#[sealed]
pub trait RestrictType: Default {}

// The given root reference cannot be sent to other thread
// NOTE: This type is considered to be part of public API
#[derive(Default)]
pub struct Unsendable {}
#[sealed]
impl RestrictType for Unsendable {}

// The given root reference can be sent to other thread
// NOTE: This type is considered to be part of public API
#[derive(Default)]
pub struct Sendable {}
#[sealed]
impl RestrictType for Sendable {}

// NOTE: This type is considered to be part of public API
pub struct RootRef<'a, Restriction: RestrictType, Kind: RefKind, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, T>,
  _kind: PhantomData<(Restriction, Kind)>
}

impl<'a, Restriction: RestrictType, Kind: RefKind, T: ObjectLikeTrait> RootRef<'a, Restriction, Kind, T> {
  pub unsafe fn new(inner: RootRefRaw<'a, T>) -> Self {
    return Self {
      inner,
      _kind: PhantomData {}
    }
  }
  
  pub fn into_raw(this: Self) -> RootRefRaw<'a, T> {
    return this.inner;
  }
}

impl<'a, Restriction: RestrictType, Kind: RefKind, T: ObjectLikeTrait> Deref for RootRef<'a, Restriction, Kind, T> {
  type Target = T;
  
  fn deref(&self) -> &Self::Target {
    // SAFETY: All root refs can be immutably borrow
    // and safety of not having another mutable reference
    // is ensure by API design of one way downgrade to shared
    // before it is sent to other thread
    return unsafe { self.inner.borrow_inner() };
  }
}

impl<'a, Restriction: RestrictType, T: ObjectLikeTrait> RootRef<'a, Restriction, Exclusive, T> {
  pub fn downgrade(this: Self) -> RootRef<'a, Restriction, Shared, T> {
    // SAFETY: This is exclusive borrow and it is safe to downgrade
    // to shared
    return unsafe { RootRef::new(this.inner) };
  }
}

impl<'a, Restriction: RestrictType, T: ObjectLikeTrait> DerefMut for RootRef<'a, Restriction, Exclusive, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    // SAFETY: Only exclusive root ref can be mutably
    // borrowed and API design ensure there no other
    // immutable borrows
    return unsafe { self.inner.borrow_inner_mut() };
  }
}

