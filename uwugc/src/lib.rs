#![deny(clippy::perf)]
#![deny(clippy::correctness)]
#![deny(clippy::suspicious)]
#![deny(clippy::pedantic)]
#![deny(clippy::complexity)]
#![deny(clippy::style)]
#![deny(clippy::as_underscore)]

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
