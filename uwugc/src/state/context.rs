use std::{
    any::Any,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::Arc,
};

use parking_lot::{Mutex, MutexGuard};

use crate::{
    TypeManager, gc_sync, mm,
    object::{Metadata, MetadataCompressed, ObjectKind, ObjectPtr},
    root_set::RootSet,
    state::{SharedState, UwUGC},
};

pub struct ContextShared {
    pub mm_context: mm::Context,
    pub root_set: Box<dyn RootSet>,
}

pub struct Context<'a, R: RootSet> {
    owner: &'a UwUGC,
    shared: gc_sync::SharedGuard<'a, SharedState>,
    shared_data: Arc<Mutex<ContextShared>>,
    _not_send_sync: PhantomData<*mut u8>,
    _phantom: PhantomData<R>,
}

#[derive(Clone, Copy)]
pub enum AllocType {
    // type id that gets passed to type manager
    Typed(u64),

    // Size in bytes, this implies drop code will
    // not be run
    PlainOldData(usize),
}

impl<'a, R> Context<'a, R>
where
    R: RootSet,
{
    pub(crate) fn new(
        owner: &'a UwUGC,
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
    pub fn alloc_fast(&mut self, ty: AllocType, extra_bytes: usize) -> Option<ObjectPtr> {
        let kind = self.to_obj_kind(ty);

        // SAFETY: We're using same mm consistently
        let ret = unsafe {
            self.shared_data
                .lock()
                .mm_context
                .alloc(&self.shared.get().mm, self.get_size_of_alloc(ty) + extra_bytes)
        };

        ret.map(|x| unsafe { Self::init_object(self, x.0, kind) })
    }

    fn to_obj_kind(&self, ty: AllocType) -> ObjectKind {
        match ty {
            AllocType::PlainOldData(x) => ObjectKind::PlainOldData(u64::try_from(x).unwrap()),
            AllocType::Typed(x) => ObjectKind::NotPlainOldData(x),
        }
    }

    fn get_size_of_alloc(&self, ty: AllocType) -> usize {
        (match ty {
            AllocType::PlainOldData(x) => x,
            AllocType::Typed(x) => self
                .shared
                .get()
                .type_manager
                .type_manager
                .get_size(x)
                .unwrap(),
        }) + size_of::<MetadataCompressed>()
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
        unsafe { ObjectPtr::from_raw(ptr) }.unwrap()
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
    pub unsafe fn alloc_slow(&mut self, ty: AllocType, extra_bytes: usize) -> Option<ObjectPtr> {
        // Retry 3 times :3
        for _ in 0..3 {
            let kind = self.to_obj_kind(ty);
            // SAFETY: We're using same mm consistently
            let ret = unsafe {
                self.shared_data
                    .lock()
                    .mm_context
                    .alloc(&self.shared.get().mm, self.get_size_of_alloc(ty) + extra_bytes)
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

    pub fn get_type_manager(&'a self) -> &'a dyn TypeManager {
        &*self.shared.get().type_manager.type_manager
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
