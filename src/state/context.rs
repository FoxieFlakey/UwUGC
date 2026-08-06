use std::{marker::PhantomData, sync::Arc};

use parking_lot::Mutex;

use crate::{
    gc_sync, mm,
    state::{SharedState, State},
};

pub struct ContextShared {
    pub mm_context: mm::Context,
}

pub struct Context<'a> {
    owner: &'a State,
    shared: gc_sync::SharedGuard<'a, SharedState>,
    shared_data: Arc<Mutex<ContextShared>>,
    _not_send_sync: PhantomData<*mut u8>,
}

// Containing all stuffs
// to perform safepoints
pub struct SafepointArgs {}

impl<'a> Context<'a> {
    pub(crate) fn new(
        owner: &'a State,
        shared: gc_sync::SharedGuard<'a, SharedState>,
        shared_data: Arc<Mutex<ContextShared>>,
    ) -> Self {
        Self {
            owner,
            shared,
            shared_data,
            _not_send_sync: PhantomData,
        }
    }

    // A safepoint, where current thread can pause
    //
    // # Safety
    // Caller MUST assume all pointers to the heap are invalidated
    // and has to be "reloaded" from root. If some pointers need to
    // stay live after this.
    //
    // DO NOTE, if you're calee. you dont know what caller might want
    // to keep. SO be VERY careful
    pub unsafe fn safepoint(&self, _safepoint_args: &SafepointArgs) {
        self.shared.safepoint();
    }

    // Allocates new object, if allocation fail. Call alloc_slow.
    // Failure of allocation on either function, schedules an GC
    //
    // In short, the pattenr should look like
    //
    // let mut obj = context.alloc_fast(8192);
    // if obj.is_none() {
    //    save_pointers_to_root();
    //    // When its none, it is already end of world. GC already tries its best
    //    obj = context.alloc_slow(8192).unwrap();
    // }
    //
    // <use the object>
    pub fn alloc_fast(&mut self, size: usize) -> Option<*mut u8> {
        return self
            .shared_data
            .lock()
            .mm_context
            .alloc(&self.shared.get().mm, size)
            .map(|x| x.0);
    }

    // This is like alloc_fast, but this may start GC/be blocked. So
    // safepoint is needed so GC knows where pointers are.
    //
    // # Safety
    // Caller MUST assume all pointers to the heap are invalidated
    // and has to be "reloaded" from root. If some pointers need to
    // stay live after this.
    //
    // DO NOTE, if you're calee. you dont know what caller might want
    // to keep. SO be VERY careful
    pub unsafe fn alloc_slow(&mut self, size: usize, _safepoint_args: &SafepointArgs) -> Option<*mut u8> {
        // Retry 3 times :3
        for _ in 0..3 {
            let ret = self
                .shared_data
                .lock()
                .mm_context
                .alloc(&self.shared.get().mm, size)
                .map(|x| x.0);

            if ret.is_some() {
                return ret;
            }

            self.shared.unguarded(|| {
                // Trigger an GC
                self.owner.controller.start_and_wait_cycle();
            })
        }

        None
    }
}
