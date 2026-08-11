use std::{
    any::Any,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use arbitrary_int::u60;
use parking_lot::{Mutex, MutexGuard};

use crate::{
    gc_sync, mm,
    object::{Metadata, MetadataCompressed, ObjectKind, ObjectPtr},
    root_set::RootSet,
    state::{SharedState, State},
};

pub struct ContextShared {
    pub mm_context: mm::Context,
    pub root_set: Box<dyn RootSet>,
}

pub struct Context<'a, R: RootSet> {
    owner: &'a State,
    shared: gc_sync::SharedGuard<'a, SharedState>,
    shared_data: Arc<Mutex<ContextShared>>,
    _not_send_sync: PhantomData<*mut u8>,
    _phantom: PhantomData<R>,
}

impl<'a, R> Context<'a, R>
where
    R: RootSet,
{
    pub(crate) fn new(
        owner: &'a State,
        shared: gc_sync::SharedGuard<'a, SharedState>,
        shared_data: Arc<Mutex<ContextShared>>,
    ) -> Self {
        Self {
            owner,
            shared,
            shared_data,
            _phantom: PhantomData,
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
    pub unsafe fn safepoint(&mut self) {
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

        // SAFETY: We're using same mm consistently
        let ret = unsafe {
            self.shared_data
                .lock()
                .mm_context
                .alloc(&self.shared.get().mm, size)
        };

        ret.map(|x| unsafe { Self::init_object(self, x.0, kind) })
    }

    // # Safety
    // caller has to ensure 'ptr' is atleast MetadataCompressed size and
    // has valid metadata which include the sizing and aligned to be
    // 64-bit on both size and alignment of pointer
    unsafe fn init_object(&self, ptr: *mut u8, kind: ObjectKind) -> ObjectPtr {
        let meta = Metadata {
            is_marked: !self.shared.get().mark_true_bit,
            payload: kind,
            write_barrier_activated: false,
        };

        // SAFETY: Caller ensure corect alignment and size
        unsafe {
            ptr.cast::<MetadataCompressed>()
                .write(MetadataCompressed::new(meta))
        };

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
    pub unsafe fn alloc_slow(&mut self, size: usize) -> Option<ObjectPtr> {
        // Retry 3 times :3
        for _ in 0..3 {
            let kind =
                ObjectKind::PlainOldData(u60::try_new(u64::try_from(size).unwrap()).unwrap());
            // SAFETY: We're using same mm consistently
            let ret = unsafe {
                self.shared_data
                    .lock()
                    .mm_context
                    .alloc(&self.shared.get().mm, size)
            };

            if ret.is_some() {
                return ret.map(|x| unsafe { Self::init_object(self, x.0, kind) });
            }

            self.shared.unguarded(|| {
                // Trigger an GC
                self.owner.controller.start_and_wait_cycle();
            })
        }

        None
    }

    pub fn get_root_set(&'a self) -> RootSetGuard<'a, R> {
        RootSetGuard {
            _not_send_sync: PhantomData,
            _phantom: PhantomData,
            guard: self.shared_data.lock(),
        }
    }
}

pub struct RootSetGuard<'a, R: RootSet> {
    guard: MutexGuard<'a, ContextShared>,
    _phantom: PhantomData<R>,
    _not_send_sync: PhantomData<*mut u8>,
}

impl<'a, R> Deref for RootSetGuard<'a, R>
where
    R: RootSet,
{
    type Target = R;

    fn deref(&self) -> &Self::Target {
        let as_any = &*self.guard.root_set as &dyn Any;
        as_any.downcast_ref().unwrap()
    }
}

impl<'a, R> DerefMut for RootSetGuard<'a, R>
where
    R: RootSet,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        let as_any = &mut *self.guard.root_set as &mut dyn Any;
        as_any.downcast_mut().unwrap()
    }
}
