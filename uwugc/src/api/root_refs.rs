// A abstraction layer which can specialize RootRef
// into various level of exclusiveness

use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};

use sealed::sealed;

use crate::allocator::HeapAlloc;
use crate::api::ObjectLikeTrait;
use crate::heap::RootRefRaw;

// Re-export for compatibility at the moment
pub use crate::access_kind::{Exclusive, Shared, RefKind};

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
pub struct RootRef<'a, Restriction: RestrictType, Kind: RefKind, A: HeapAlloc, T: ObjectLikeTrait> {
  inner: RootRefRaw<'a, A, T>,
  _kind: PhantomData<(Restriction, Kind)>
}

impl<'a, Restriction: RestrictType, Kind: RefKind, A: HeapAlloc, T: ObjectLikeTrait> RootRef<'a, Restriction, Kind, A, T> {
  pub(crate) unsafe fn new(inner: RootRefRaw<'a, A, T>) -> Self {
    Self {
      inner,
      _kind: PhantomData {}
    }
  }
  
  pub(crate) fn into_raw(this: Self) -> RootRefRaw<'a, A, T> {
    this.inner
  }
}

impl<Restriction: RestrictType, Kind: RefKind, A: HeapAlloc, T: ObjectLikeTrait> Deref for RootRef<'_, Restriction, Kind, A, T> {
  type Target = T;
  
  fn deref(&self) -> &Self::Target {
    // SAFETY: All root refs can be immutably borrow
    // and safety of not having another mutable reference
    // is ensure by API design of one way downgrade to shared
    // before it is sent to other thread
    unsafe { self.inner.borrow_inner() }
  }
}

impl<'a, Restriction: RestrictType, A: HeapAlloc, T: ObjectLikeTrait> RootRef<'a, Restriction, Exclusive, A, T> {
  #[must_use]
  pub fn downgrade(this: Self) -> RootRef<'a, Restriction, Shared, A, T> {
    // SAFETY: This is exclusive borrow and it is safe to downgrade
    // to shared
    unsafe { RootRef::new(this.inner) }
  }
}

impl<Restriction: RestrictType, A: HeapAlloc, T: ObjectLikeTrait> DerefMut for RootRef<'_, Restriction, Exclusive, A, T> {
  fn deref_mut(&mut self) -> &mut Self::Target {
    // SAFETY: Only exclusive root ref can be mutably
    // borrowed and API design ensure there no other
    // immutable borrows
    unsafe { self.inner.borrow_inner_mut() }
  }
}

