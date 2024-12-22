// Public facing, stable API for UwUGC

pub(crate) mod helper;

use std::sync::Arc;

use crate::heap::Context as HeapContext;
use crate::heap::Heap as HeapInternal;

pub use crate::heap::Params;
pub use crate::gc::GCParams;
pub use crate::descriptor::{Describeable, Descriptor, Field};
pub use crate::objects_manager::ObjectLikeTrait;
pub use crate::heap::ConstructorScope;

pub mod root_refs;

mod gc_box;
pub use gc_box::GCBox;

helper::export_type_as_wrapper!(HeapArc, Arc<HeapInternal>);
mod heap;

helper::export_type_as_wrapper!('a, Context, HeapContext<'a>);
mod context;



