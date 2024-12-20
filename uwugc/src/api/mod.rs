// Public facing, stable API for UwUGC

pub(crate) mod helper;

use std::sync::Arc;

use crate::heap::context::ContextHandle as HeapContextHandle;
use crate::heap::Heap as HeapInternal;

pub use crate::heap::HeapParams;
pub use crate::gc::GCParams;
pub use crate::descriptor::{Describeable, Descriptor, Field};
pub use crate::objects_manager::ObjectLikeTrait;
pub use crate::heap::context::ObjectConstructorContext;

// Temporary exports
pub use crate::root_refs::{RootRef, RefKind, Shared, Exclusive, RestrictType, Sendable, Unsendable};

helper::export_type_as_wrapper!(HeapArc, Arc<HeapInternal>);
mod heap;

helper::export_type_as_wrapper!('a, Context, HeapContextHandle<'a>);
mod context;



