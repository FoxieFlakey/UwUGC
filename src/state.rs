use std::{
    collections::HashMap,
    mem::ManuallyDrop,
    sync::{Arc, OnceLock},
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;
use thiserror::Error;

use crate::{
    gc,
    gc_controller::GCController,
    gc_sync::{self, GCSync},
    mm::{self, MM},
    root_set::{RootSet, RootSetRaw},
    state::context::{Context, ContextShared},
};

mod context;
pub use context::SafepointArgs;

pub struct SharedState {
    pub mm: MM,
    pub gc_state: OnceLock<gc::PersistentState>,
    pub contexts: Mutex<HashMap<ThreadId, Arc<Mutex<ContextShared>>>>,
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

        let root_set = Arc::new(root_set_maker(RootSetRaw::new(root_set_size)));
        let shared_data = Arc::new(Mutex::new(ContextShared {
            mm_context: mm::Context::new(),
            root_set: root_set.clone(),
        }));

        let shared = self.shared.get_shared();
        shared
            .get()
            .contexts
            .lock()
            .insert(thread::current_id(), shared_data.clone());

        Context::new(self, shared, shared_data, root_set)
    }

    pub fn new(size: usize) -> Result<State, CreateError> {
        let shared = Arc::new(
            GCSync::new(SharedState {
                mm: MM::new(size)?,
                contexts: Mutex::new(HashMap::new()),
                gc_state: OnceLock::new(),
            })
            .map_err(|x| x.0)?,
        );
        let controller = Arc::new(GCController::new());

        Ok(State {
            shared: shared.clone(),
            controller: controller.clone(),
            gc_thread: ManuallyDrop::new(thread::spawn(move || gc_thread(shared, controller))),
        })
    }
}

fn gc_thread(shared: Arc<GCSync<SharedState>>, controller: Arc<GCController>) {
    println!("[GC] Started");

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
        gc::do_cycle(&shared, &controller);
        println!("[GC] Cycle end");
    });

    println!("[GC] Shutdown");
}
