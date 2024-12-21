#![feature(box_as_ptr)]

mod api;
mod objects_manager;
mod heap;
mod gc;
mod descriptor;
mod refs;

// Publicize the API
pub use api::*;
