// This assume each thread has own root set

use std::{cell::RefCell, marker::PhantomData, sync::Arc};

use bitvec::vec::BitVec;
use uwugc::{ObjectPtr, UwUGC};
use yoke::{Yoke, Yokeable};

use crate::{UwUGCPlus, types::TypeId};

#[derive(Clone)]
struct Inner {
    set: Vec<Option<ObjectPtr>>,
    used_map: BitVec,

    // number of root references that is
    // "stored" so safepoint can be performed
    stored_count: usize,
    exist_count: usize,
}

#[derive(Yokeable)]
pub struct Context<'a> {
    context: uwugc::Context<'a, Inner>,
}

pub struct RootRefRaw {
    ptr: Option<ObjectPtr>,
    idx: usize,

    // RootRef is right belong'ed to one thread
    // cannot be transferred to other.
    _no_send_sync: PhantomData<*mut u8>,
}

impl Drop for RootRefRaw {
    fn drop(&mut self) {
        let idx = self.idx;
        let is_stored = self.ptr.is_none();
        with_context_mut(move |x| {
            let mut root_set = x.context.get_root_set();
            root_set.exist_count -= 1;

            if is_stored {
                root_set.stored_count -= 1;
            }

            // Remove reference from root
            root_set.set[idx] = None;
            root_set.used_map.set(idx, false);
        });
    }
}

impl RootRefRaw {
    pub fn get_ptr(&self) -> ObjectPtr {
        self.ptr
            .expect("This root ref is stored to root set, please load first")
    }

    pub fn store(&mut self) {
        let ptr = self.ptr.take().unwrap();
        let idx = self.idx;
        with_context_mut(move |x| {
            let mut root_set = x.context.get_root_set();
            root_set.stored_count += 1;

            // Each slot can only be used by one RootRefRaw
            assert!(
                root_set.set[idx].is_none(),
                "Same index is reused when it must not"
            );
            root_set.set[idx] = Some(ptr);
        });
    }

    pub fn load(&mut self) {
        let idx = self.idx;
        self.ptr = Some(with_context_mut(move |x| {
            let mut root_set = x.context.get_root_set();
            root_set.stored_count -= 1;

            // Each slot can only be used by one RootRefRaw. So it should not
            // be already loaded
            assert!(root_set.set[idx].is_some(), "Slot is already loaded?");
            root_set.set[idx].take().unwrap()
        }));
    }
}

impl<'a> Context<'a> {
    pub fn new(uwugc: &'a UwUGC) -> Self {
        Self {
            context: uwugc.new_context(Inner {
                exist_count: 0,
                stored_count: 0,
                set: Vec::new(),
                used_map: BitVec::new(),
            }),
        }
    }

    pub fn add_ptr<'b>(&'b mut self, ptr: ObjectPtr) -> RootRefRaw
    where
        'a: 'b,
    {
        let mut inner = self.context.get_root_set();
        let Some(free_index) = inner.used_map.first_zero() else {
            let index = inner.set.len();
            inner.set.insert(index, None);
            inner.used_map.push(true);
            inner.exist_count += 1;

            return RootRefRaw {
                idx: index,
                ptr: Some(ptr),
                _no_send_sync: PhantomData,
            };
        };

        assert!(inner.set[free_index].is_none());
        inner.set[free_index] = None;
        inner.used_map.set(free_index, true);
        inner.exist_count += 1;

        RootRefRaw {
            idx: free_index,
            ptr: Some(ptr),
            _no_send_sync: PhantomData,
        }
    }

    pub fn alloc_fast(&mut self, type_id: TypeId, extra_bytes: usize) -> Option<RootRefRaw> {
        self.context
            .alloc_fast(uwugc::AllocType::Typed(type_id.0), extra_bytes)
            .map(|x| self.add_ptr(x))
    }

    pub fn alloc_slow(&mut self, type_id: TypeId, extra_bytes: usize) -> Option<RootRefRaw> {
        let root_set = self.context.get_root_set();
        assert!(
            root_set.stored_count == root_set.exist_count,
            "There stil active RootReference, store them before calling alloc_slow (its implicitly also safepoint)"
        );
        drop(root_set);

        // SAFETY: We checked that all root references are stored first
        unsafe { self.context.alloc_slow(uwugc::AllocType::Typed(type_id.0), extra_bytes) }
            .map(|x| self.add_ptr(x))
    }

    // This panics if there any active root reference
    pub fn safepoint(&mut self) {
        let root_set = self.context.get_root_set();
        assert!(
            root_set.stored_count == root_set.exist_count,
            "There stil active RootReference, store them before safepoint"
        );
        drop(root_set);

        // SAFETY: We make sure there no living root reference
        unsafe { self.context.safepoint() };
    }
}

unsafe impl uwugc::RootSet for Inner {
    fn clone_boxed(&mut self) -> Box<dyn uwugc::RootSet> {
        Box::new(self.clone())
    }

    fn iter_pointers(&mut self, visitor: &mut dyn FnMut(&ObjectPtr)) {
        self.set.iter().flatten().for_each(visitor);
    }

    fn map_pointers(&mut self, visitor: &mut dyn FnMut(ObjectPtr) -> ObjectPtr) {
        self.set.iter_mut().flatten().for_each(|x| *x = visitor(*x));
    }
}

thread_local! {
    pub static CURRENT_CONTEXT: RefCell<Option<Yoke<Context<'static>, Arc<UwUGCPlus>>>> = RefCell::new(None);
}

pub fn with_context_mut<F, R>(func: F) -> R
where
    F: FnOnce(&mut Context<'_>) -> R + 'static,
    R: 'static,
{
    CURRENT_CONTEXT.with_borrow_mut(|x| {
        x.as_mut()
            .expect("Current thread has no associated context")
            .with_mut_return(func)
    })
}
