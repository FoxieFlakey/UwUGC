#![forbid(clippy::perf)]
#![forbid(clippy::correctness)]
#![forbid(clippy::suspicious)]
#![forbid(clippy::pedantic)]
#![forbid(clippy::complexity)]
#![forbid(clippy::style)]
#![forbid(clippy::as_underscore)]
#![forbid(clippy::as_conversions)]
#![forbid(unsafe_op_in_unsafe_fn)]

#![feature(alloc_layout_extra)]
#![feature(ptr_as_uninit)]
#![feature(non_null_from_ref)]

mod api;
mod objects_manager;
mod heap;
mod gc;
mod descriptor;
mod refs;
mod allocator;
mod driver;

// Publicize the API
pub use api::*;
