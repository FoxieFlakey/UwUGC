use std::{
    collections::HashMap,
    mem::ManuallyDrop,
    sync::Arc,
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;
use thiserror::Error;

use crate::{
    gc::{self, GCArgs},
    gc_controller::GCController,
    gc_sync::{self, GCSync},
    mm::{self, MM},
    object::Bit,
    type_manager::{TypeManagerConcrete, TypeManager},
    root_set::{RootSet, RootSetRaw},
    state::context::{Context, ContextShared},
};

mod context;

pub struct SharedState {
    pub mm: MM,
    pub contexts: Mutex<HashMap<ThreadId, Arc<Mutex<ContextShared>>>>,
    pub mark_true_bit: Bit,
    pub offset_walker: TypeManagerConcrete,
}
pub struct State {
    controller: Arc<GCController>,
    shared: Arc<GCSync<SharedState>>,
    gc_thread: ManuallyDrop<JoinHandle<()>>,
}

impl Drop for State {
    fn drop(&mut self) {
        self.controller.shutdown();
        // WE want to wait the GC thread
        unsafe { ManuallyDrop::take(&mut self.gc_thread) }
            .join()
            .unwrap();
    }
}

#[derive(Error, Debug)]
pub enum CreateError {
    #[error("cannot init memory management")]
    CreateMM(
        #[from]
        #[source]
        mm::CreateError,
    ),

    #[error("cannot create GCSync")]
    CreateGCSync(
        #[from]
        #[source]
        gc_sync::CreateError,
    ),
}

impl State {
    // There has to be only one context per thread!
    // or else there contexts that "cant" be parked
    // or safepoint'ed so GC can be deadlocked
    pub fn new_context<'a, F, T>(
        &'a self,
        root_set_size: usize,
        root_set_maker: F,
    ) -> Context<'a, T>
    where
        T: RootSet + 'static,
        F: FnOnce(RootSetRaw) -> T,
    {
        let root_set_size = if root_set_size.is_multiple_of(page_size::get()) {
            root_set_size
        } else {
            root_set_size.next_multiple_of(page_size::get())
        };

        let shared_data = Arc::new(Mutex::new(ContextShared {
            mm_context: mm::Context::new(),
            root_set: Box::new(root_set_maker(RootSetRaw::new(root_set_size))),
        }));

        let shared = self.shared.get_shared();
        shared
            .get()
            .contexts
            .lock()
            .insert(thread::current_id(), shared_data.clone());

        Context::new(self, shared, shared_data)
    }

    pub fn new<M: TypeManager + 'static>(
        size: usize,
        preferred_heap_base: Option<usize>,
        preferred_temp_base: Option<usize>,
        descriptor_manager: M,
    ) -> Result<State, CreateError> {
        let shared = Arc::new(
            GCSync::new(SharedState {
                mm: MM::new(size, preferred_heap_base)?,
                contexts: Mutex::new(HashMap::new()),
                mark_true_bit: Bit::Bit1,
                offset_walker: TypeManagerConcrete::new(descriptor_manager),
            })
            .map_err(|x| x.0)?,
        );
        let controller = Arc::new(GCController::new());

        let gc_args = GCArgs {
            preferred_temp_base,
        };

        Ok(State {
            shared: shared.clone(),
            controller: controller.clone(),
            gc_thread: ManuallyDrop::new(thread::spawn(move || {
                gc_thread(shared, controller, gc_args)
            })),
        })
    }
}

fn gc_thread(shared: Arc<GCSync<SharedState>>, controller: Arc<GCController>, gc_args: GCArgs) {
    println!("[GC] Started");

    let mut gc_state = None;
    controller.do_looper(|| {
        println!("[GC] Flushing contexts");
        // Flushing necessary because each mm context keep track
        // of current cached FlexPage. but after GC freeing spaces
        // or moves around. Those FlexPages are not valid anymore
        shared
            .get_exclusive()
            .get()
            .contexts
            .get_mut()
            .iter_mut()
            .map(|x| x.1.lock())
            .for_each(|mut x| {
                x.mm_context.flush_local_buf();
            });

        println!("[GC] Cycle start");
        gc_state = Some(gc::do_cycle(&shared, &controller, &gc_args, gc_state.take()));
        println!("[GC] Cycle end");
    });

    println!("[GC] Shutdown");
}
