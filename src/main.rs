#![feature(current_thread_id)]

use crate::{
    root_set::NoopRootSet,
    state::{SafepointArgs, State},
};

mod bitmap;
mod gc;
mod gc_controller;
mod gc_sync;
mod mm;
mod mmap;
mod object;
mod pipe;
mod root_set;
mod state;

fn main() {
    println!("Hello, world!");

    let state = State::new(128 * 1024 * 1024).unwrap();

    let mut ctx = state.new_context(8192, NoopRootSet::new);
    let safepoint_args = SafepointArgs {};

    loop {
        let _ = ctx
            .alloc_fast(8192)
            .or_else(|| {
                // This comment can be like safepoint'ing stuffs
                // spilling contents and such
                unsafe { ctx.alloc_slow(8192, &safepoint_args) }
            })
            .unwrap();

        unsafe { ctx.safepoint(&safepoint_args) };
    }
}
