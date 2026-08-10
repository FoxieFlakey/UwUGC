#![feature(current_thread_id)]

use std::slice;

use crate::{
    object::ObjectPtr, root_set::{RootSet, RootSetRaw}, state::State
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

    let mut ctx = state.new_context(8192, DumbRootSet::new);
    let obj = ctx
        .alloc_fast(8192)
        .or_else(|| {
            // This comment can be like safepoint'ing stuffs
            // spilling contents and such
            unsafe { ctx.alloc_slow(8192) }
        })
        .unwrap();
    let mut set = ctx.get_root_set();

    set.as_slice_mut()[0] = Some(obj);
    drop(set);

    loop {
        let _ = ctx
            .alloc_fast(8192)
            .or_else(|| {
                // This comment can be like safepoint'ing stuffs
                // spilling contents and such
                unsafe { ctx.alloc_slow(8192) }
            })
            .unwrap();

        unsafe { ctx.safepoint() };
    }
}

pub struct DumbRootSet {
    raw: RootSetRaw,
    len: usize,
}

impl DumbRootSet {
    pub fn new(set: RootSetRaw) -> Self {
        Self {
            len: set.get_size() / size_of::<Option<ObjectPtr>>(),
            raw: set,
        }
    }

    // shared reference ensures no mutable reference to the memory
    pub fn as_slice<'a>(&'a self) -> &'a [Option<ObjectPtr>] {
        unsafe { slice::from_raw_parts(self.raw.get_ptr().cast(), self.len) }
    }

    // &mut ensure nothing accesses the memory
    pub fn as_slice_mut<'a>(&'a mut self) -> &'a mut [Option<ObjectPtr>] {
        unsafe { slice::from_raw_parts_mut(self.raw.get_ptr().cast(), self.len) }
    }
}

unsafe impl RootSet for DumbRootSet {
    fn clone_metadata(&self, set: RootSetRaw) -> Box<dyn RootSet> {
        Box::new(Self {
            raw: set,
            len: self.len,
        })
    }

    fn get_raw(&self) -> &RootSetRaw {
        &self.raw
    }

    fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr)) {
        self.as_slice()
            .iter()
            .flatten()
            .for_each(visitor);
    }

    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {
        self.as_slice_mut()
            .iter_mut()
            .flatten()
            .for_each(|x| {
                *x = visitor(*x);
            });
    }
}


