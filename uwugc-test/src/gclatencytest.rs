use std::{
    mem::MaybeUninit, ptr::NonNull, slice, sync::atomic::{AtomicPtr, Ordering}, time::Instant
};

use uwugc::{AllocType, Context, ObjectPtr, RootSet, RootSetRaw, TypeManager, UwUGC};

// Ported from https://github.com/WillSewell/gc-latency-experiment/blob/f67121ec8a741201414c76d5ba85f9304c774acc/java/Main.java
// its Java version that ported

const WINDOW_TYPE_ID: u64 = 0;
const WINDOW_SIZE: usize = 200_000;
const MSG_COUNT: usize = 10_000_000;
const MSG_SIZE: usize = 1024;

fn make_message(ctx: &mut Context<'_, DumbRootSet>, n: u8) -> Option<ObjectPtr> {
    ctx.alloc_fast(AllocType::PlainOldData(MSG_SIZE))
        .or_else(|| {
            // SAFETY: Dont have anything imporant to save into root set
            unsafe { ctx.alloc_slow(AllocType::PlainOldData(MSG_SIZE)) }
        })
        .inspect(|x| {
            // SAFETY: We allocated MSG_SIZE bytes
            let slice =
                unsafe { slice::from_raw_parts_mut(x.data().cast::<MaybeUninit<u8>>().as_ptr(), MSG_SIZE) };
            slice.fill(MaybeUninit::new(n));
        })
}

fn push_message(ctx: &mut Context<'_, DumbRootSet>, id: usize) {
    let message = make_message(ctx, (id & 0xFF) as u8).unwrap();

    let root = ctx.get_root_set();
    let window = get_window(&root);

    window[id % WINDOW_SIZE].store(message.into_raw().as_ptr(), Ordering::Relaxed);
}

pub fn run(state: &mut UwUGC) {
    state.set_type_manager(LatencyTestTypeManager);
    let state = &state;

    let mut ctx = state.new_context(8192, DumbRootSet::new);
    let window = ctx.alloc_fast(AllocType::Typed(WINDOW_TYPE_ID)).unwrap();
    ctx.get_root_set().as_slice_mut()[0] = Some(window);

    let mut worst = None;

    for id in 0..MSG_COUNT {
        let start = Instant::now();
        push_message(&mut ctx, id);
        let elapsed = start.elapsed();
        if let Some(worst) = worst.as_mut() {
            if elapsed > *worst {
                *worst = elapsed;
            }
        } else {
            worst = Some(elapsed);
        }

        // SAFETY: We dont need anything special to save
        unsafe { ctx.safepoint() };
    }

    if let Some(worst) = worst {
        println!("Worst push time: {:6.2} ms", worst.as_millis() as f64);
    } else {
        println!("Nothing is ran");
    }
}

fn get_window<'a>(ctx: &'a DumbRootSet) -> &'a [AtomicPtr<u8>] {
    let data = ctx.as_slice()[0].unwrap().data().as_ptr().cast::<AtomicPtr<u8>>();

    // SAFETY: the type ID is corect
    unsafe { slice::from_raw_parts(data, WINDOW_SIZE) }
}

pub struct LatencyTestTypeManager;

unsafe impl TypeManager for LatencyTestTypeManager {
    fn assume_all_dead(&mut self) {}
    fn wipe_deads(&mut self) {}

    fn set_alive(&self, type_id: u64) -> bool {
        type_id == WINDOW_TYPE_ID
    }

    fn get_size(&self, type_id: u64) -> Option<usize> {
        if type_id == WINDOW_TYPE_ID {
            Some(size_of::<AtomicPtr<u8>>() * WINDOW_SIZE)
        } else {
            None
        }
    }

    fn iterate_gc_pointers(
        &self,
        type_id: u64,
        object: ObjectPtr,
        visitor: &mut dyn FnMut(ObjectPtr),
    ) -> bool {
        if type_id != WINDOW_TYPE_ID {
            return false;
        }
        let data = object.data().as_ptr().cast::<AtomicPtr<u8>>();

        // SAFETY: the type ID is corect
        let slice = unsafe { slice::from_raw_parts(data, WINDOW_SIZE) };
        for ptr in slice {
            if let Some(ptr) = NonNull::new(ptr.load(Ordering::Relaxed)) {
                // SAFETY: We only ever store valid object pointer in this array
                visitor(unsafe { ObjectPtr::from_nonnull(ptr) });
            }

        }

        true
    }

    fn update_gc_pointers(
        &self,
        type_id: u64,
        object: ObjectPtr,
        updater: &mut dyn FnMut(ObjectPtr) -> ObjectPtr,
    ) -> bool {
        if type_id != WINDOW_TYPE_ID {
            return false;
        }

        let data = object.data().cast::<AtomicPtr<u8>>().as_ptr();

        // SAFETY: the type ID is corect
        let slice = unsafe { slice::from_raw_parts(data, WINDOW_SIZE) };
        for ptr in slice {
            if let Some(ptr_loaded) = NonNull::new(ptr.load(Ordering::Relaxed)) {
                // SAFETY: We only ever store valid object pointer in this array
                let updated = updater(unsafe { ObjectPtr::from_nonnull(ptr_loaded) });
                ptr.store(updated.into_raw().as_ptr(), Ordering::Relaxed);
            }
        }

        true
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



