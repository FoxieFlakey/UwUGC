use std::{marker::PhantomData, sync::Arc};

use arbitrary_int::u60;
use parking_lot::Mutex;

use crate::{
    gc_sync, mm, object::{Metadata, MetadataCompressed, ObjectKind, ObjectPtr}, root_set::RootSet, state::{SharedState, State}
};

pub struct ContextShared {
    pub mm_context: mm::Context,
    pub root_set: Arc<dyn RootSet>,
}

pub struct Context<'a, R: RootSet> {
    owner: &'a State,
    shared: gc_sync::SharedGuard<'a, SharedState>,
    shared_data: Arc<Mutex<ContextShared>>,
    root_set_concrete: Arc<R>,
    _not_send_sync: PhantomData<*mut u8>,
}

// Containing all stuffs
// to perform safepoints
pub struct SafepointArgs {}

impl<'a, R> Context<'a, R>
where
    R: RootSet,
{
    pub(crate) fn new(
        owner: &'a State,
        shared: gc_sync::SharedGuard<'a, SharedState>,
        shared_data: Arc<Mutex<ContextShared>>,
        root_set_concrete: Arc<R>,
    ) -> Self {
        Self {
            owner,
            shared,
            shared_data,
            root_set_concrete,
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
    pub fn alloc_fast(&mut self, size: usize) -> Option<ObjectPtr> {
        let kind = ObjectKind::PlainOldData(u60::try_new(u64::try_from(size).unwrap()).unwrap());
        return self
            .shared_data
            .lock()
            .mm_context
            .alloc(&self.shared.get().mm, size)
            .map(|x| unsafe { Self::init_object(x.0, kind) });
    }

    // # Safety
    // caller has to ensure 'ptr' is atleast MetadataCompressed size and
    // has valid metadata which include the sizing and aligned to be
    // 64-bit on both size and alignment of pointer
    unsafe fn init_object(ptr: *mut u8, kind: ObjectKind) -> ObjectPtr {
        let meta = Metadata {
            is_marked: false,
            payload: kind,
            write_barrier_activated: false,
        };

        // SAFETY: Caller ensure corect alignment and size
        unsafe { ptr.cast::<MetadataCompressed>().write(MetadataCompressed::new(meta)) };

        // SAFETY: We have initialized the object to be valid object and has correct alignment and size
        unsafe { ObjectPtr::new(ptr) }
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
    pub unsafe fn alloc_slow(
        &mut self,
        size: usize,
        _safepoint_args: &SafepointArgs,
    ) -> Option<ObjectPtr> {
        // Retry 3 times :3
        for _ in 0..3 {
            let kind = ObjectKind::PlainOldData(u60::try_new(u64::try_from(size).unwrap()).unwrap());
            let ret = self
                .shared_data
                .lock()
                .mm_context
                .alloc(&self.shared.get().mm, size)
                .map(|x| unsafe { Self::init_object(x.0, kind) });

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

    #[expect(unused)]
    pub fn get_root_set(&'a self) -> &'a R {
        &self.root_set_concrete
    }
}
