// Public facing, stable API for UwUGC

pub(crate) mod helper;

use std::sync::Arc;

use crate::allocator::GlobalHeap;
use crate::heap::Context as HeapContext;
use crate::heap::Heap as HeapInternal;

pub use crate::heap::Params;
pub use crate::gc::GCParams;
pub use crate::descriptor::{Describeable, DescriptorAPI as Descriptor, Field};
pub use crate::heap::ConstructorScope;

pub mod root_refs;

mod gc_box;
pub use gc_box::GCBox;

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


