#![feature(current_thread_id)]
#![feature(duration_millis_float)]

mod bitmap;
mod gc;
mod gc_controller;
mod gc_sync;
mod mm;
mod mmap;
mod object;
mod pipe;
mod profiler;
mod quirks;
mod root_set;
mod state;
mod type_manager;

pub use object::ObjectPtr;
pub use root_set::RootSet;
pub use state::AllocType;
pub use state::Context;
pub use state::UwUGC;
pub use state::RootSetGuard;
pub use type_manager::TypeManager;
