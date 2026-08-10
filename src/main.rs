#![feature(current_thread_id)]

use std::slice;

use crate::{
    object::ObjectPtr, root_set::{RootSet, RootSetRaw}, state::{SafepointArgs, State}
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
    let safepoint_args = SafepointArgs {};
    let obj = ctx
        .alloc_fast(8192)
        .or_else(|| {
            // This comment can be like safepoint'ing stuffs
            // spilling contents and such
            unsafe { ctx.alloc_slow(8192, &safepoint_args) }
        })
        .unwrap();
    let set = ctx.get_root_set();

    // Because only this thread or GC thread access this, and it is
    // exclusive to this thread. As long as this thread is not in safepoint
    unsafe { set.as_slice_mut_unsafe()[0] = Some(obj) };
    drop(set);

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

    // # Safety
    // Caller has to ensure there no possibility of &mut exists anywhere else
    // on the backing memory for ObjectPtr
    pub unsafe fn as_slice<'a>(&'a self) -> &'a [Option<ObjectPtr>] {
        unsafe { slice::from_raw_parts(self.raw.get_ptr().cast(), self.len) }
    }

    // # Safety
    // Caller has to ensure there no possibility of &mut or & exists anywhere else
    // on the backing memory for ObjectPtr
    pub unsafe fn as_slice_mut_unsafe<'a>(&'a self) -> &'a mut [Option<ObjectPtr>] {
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

    unsafe fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr)) {
        // SAFETY: Caller made sure there nothing else uses this RootSet
        unsafe { self.as_slice() }
            .iter()
            .flatten()
            .for_each(visitor);
    }

    unsafe fn map_pointers(&self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {
        // SAFETY: Caller made sure there nothing else uses this RootSet
        unsafe { self.as_slice_mut_unsafe() }
            .iter_mut()
            .flatten()
            .for_each(|x| {
                *x = visitor(*x);
            });
    }
}


