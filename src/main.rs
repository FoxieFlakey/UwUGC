#![feature(current_thread_id)]
#![feature(duration_millis_float)]

use std::slice;

use crate::{
    object::ObjectPtr,
    root_set::{RootSet, RootSetRaw},
    state::State,
};

mod bitmap;
mod gc;
mod gc_controller;
mod gc_sync;
mod mm;
mod mmap;
mod object;
mod pipe;
mod profiler;
mod root_set;
mod state;

fn main() {
    println!("Hello, world!");

    let state = State::new(128 * 1024 * 1024).unwrap();

    let mut ctx = state.new_context(8192, DumbRootSet::new);
    let mut has_slow_pathed = false;
    loop {
        let _ = ctx
            .alloc_fast(8192)
            .or_else(|| {
                // This comment can be like safepoint'ing stuffs
                // spilling contents and such
                let ret = unsafe { ctx.alloc_slow(8192) };

                // Reload poiner as needed
                let set = ctx.get_root_set();
                let obj = set.as_slice()[0];
                println!(
                    "[Mutator] After safepoint time: 0x{:016x}",
                    obj.map(|x| x.to_ptr().addr()).unwrap_or(0)
                );
                drop(set);

                has_slow_pathed = true;
                ret
            })
            .unwrap();

        if has_slow_pathed {
            has_slow_pathed = false;
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
            println!("[Mutator] At alloc time: 0x{:016x}", obj.to_ptr().addr());
        }
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

    fn get_raw_mut(&mut self) -> &mut RootSetRaw {
        &mut self.raw
    }

    fn iter_pointers(&self, visitor: &mut dyn FnMut(&ObjectPtr)) {
        self.as_slice().iter().flatten().for_each(visitor);
    }

    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {
        self.as_slice_mut().iter_mut().flatten().for_each(|x| {
            *x = visitor(*x);
        });
    }
}
