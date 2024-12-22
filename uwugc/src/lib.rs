#![forbid(clippy::perf)]
#![forbid(clippy::correctness)]
#![forbid(clippy::suspicious)]
#![forbid(clippy::pedantic)]
#![forbid(clippy::complexity)]
#![deny(clippy::style)]
#![forbid(clippy::as_underscore)]
#![forbid(clippy::as_conversions)]
#![forbid(unsafe_op_in_unsafe_fn)]

#![allow(clippy::missing_safety_doc)]
#![allow(clippy::needless_return)]
#![feature(box_as_ptr)]

mod api;
mod objects_manager;
mod heap;
mod gc;
mod descriptor;
mod refs;

// Publicize the API
pub use api::*;
