// Public facing, stable API for UwUGC

pub(crate) mod helper;

use std::ptr;
use std::sync::Arc;

use crate::heap::Context as HeapContext;
use crate::heap::Heap as HeapInternal;

pub use crate::heap::Params;
pub use crate::gc::{GCParams, CycleStat, GCStats, CycleState, CycleStep};
pub use crate::descriptor::{Describeable, DescriptorAPI as Descriptor, Field};
pub use crate::heap::ConstructorScope;
pub use crate::allocator::GlobalHeap;

pub mod root_refs;

mod gc_box;
pub use gc_box::GCBox;
pub use gc_box::GCNullableBox;
use sealed::sealed;

helper::export_type_as_wrapper!(HeapArc, Arc<HeapInternal<GlobalHeap>>);
mod heap;

helper::export_type_as_wrapper!('a, Context, HeapContext<'a, GlobalHeap>);
mod context;

// What the data type need to implement before it is
// adequate for GC system to use
pub trait ObjectLikeTrait: 'static {
}

impl<T: 'static> ObjectLikeTrait for T {
}

// Internal trait used by crate, to include 
// internal stuffs
pub(crate) trait ObjectLikeTraitInternal: ObjectLikeTrait + 'static {
  fn drop_helper(this: *mut ());
}

impl<T: ObjectLikeTrait + 'static> ObjectLikeTraitInternal for T {
  fn drop_helper(this: *mut ()) {
    // SAFETY: The GC ensures that 'this' is correct type to be casted
    // as T because the drop_helper 
    unsafe { ptr::drop_in_place(this.cast::<T>()); }
  }
}

/// Used to mark types which can be safely transmuted/reinterpreted
/// as AtomicPtr<Object>
///
/// # Safety
///
/// The types implement this must be #[repr(transparent)]
/// and must be safe to have same bit validity as AtomicPtr<Object>
#[sealed]
pub unsafe trait ReferenceType: 'static {
}

// SAFETY: GCBox boils down to AtomicPtr<Object> and has
// #[repr(transparent)]
#[sealed]
unsafe impl<T: ObjectLikeTrait> ReferenceType for GCBox<T> {}

// SAFETY: GCNullableBox boils down to AtomicPtr<Object> and has
// #[repr(transparent)]
#[sealed]
unsafe impl<T: ObjectLikeTrait> ReferenceType for GCNullableBox<T> {}


