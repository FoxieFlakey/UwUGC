use allocator_api2::alloc::{Allocator, Global};

// An allocator suitable for allocating objects
// for GC use
pub trait HeapAlloc: Allocator + Send + Sync + 'static {
}

impl HeapAlloc for Global {}

pub type GlobalHeap = Global;

