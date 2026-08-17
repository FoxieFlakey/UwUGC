use std::{
    collections::HashMap,
    mem::{self, ManuallyDrop},
    sync::{Arc, mpsc},
    thread::{self, JoinHandle, ThreadId},
};

use parking_lot::Mutex;
use thiserror::Error;

use crate::{
    gc,
    gc_controller::GCController,
    gc_sync::{self, GCSync},
    mm::{self, MM},
    object::Bit,
    root_set::RootSet,
    type_manager::{TypeManager, TypeManagerConcrete},
};

mod context;

pub use context::{AllocType, Context, ContextShared, RootSetGuard};

pub struct SharedState {
    // This would be from-space
    pub mm: MM,

    // and this would be to-space
    // mm and second_mm are swapped as needed
    // GC may take this out
    //
    // Queue of second_mm that is usable
    second_mm: Mutex<mpsc::Receiver<MM>>,
    second_mm_done: mpsc::Sender<MM>,
    pub contexts: Mutex<HashMap<ThreadId, Arc<Mutex<ContextShared>>>>,
    pub mark_true_bit: Bit,
    pub type_manager: TypeManagerConcrete,
}

impl SharedState {
    pub fn deque_cleared_mm(&mut self) -> MM {
        self.second_mm.get_mut().recv().unwrap()
    }

    pub fn enqueue_to_be_cleared_mm(&self, mut mm: MM) {
        mm.clear();
        self.second_mm_done.send(mm).unwrap();
    }
}

pub struct UwUGC {
    controller: Arc<GCController>,
    shared: Arc<GCSync<SharedState>>,
    gc_thread: ManuallyDrop<JoinHandle<()>>,
}

impl Drop for UwUGC {
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

impl UwUGC {
    pub fn set_type_manager<T>(&mut self, new_manager: T) -> Box<dyn TypeManager>
    where
        T: TypeManager,
    {
        let new = TypeManagerConcrete::new(new_manager);

        // With &mut can be sure no other contexts exists, so shouldnt deadlock
        // This is kept becaue GC potentially still mid cycle because &mut does
        // not mean GC is not running.
        self.controller.run_exclusive(|| {
            let old = mem::replace(&mut self.shared.get_exclusive().get().type_manager, new);
            old.type_manager
        })
    }

    // There has to be only one context per thread!
    // or else there contexts that "cant" be parked
    // or safepoint'ed so GC can be deadlocked
    pub fn new_context<'a, T>(&'a self, root_set: T) -> Context<'a, T>
    where
        T: RootSet + 'static,
    {
        let shared_data = Arc::new(Mutex::new(ContextShared {
            mm_context: mm::Context::new(),
            root_set: Box::new(root_set) as Box<dyn RootSet>,
        }));

        let shared = self.shared.get_shared();
        shared
            .get()
            .contexts
            .lock()
            .insert(thread::current_id(), shared_data.clone());

        Context::new(self, shared, shared_data)
    }

    pub fn new<M: TypeManager>(
        size: usize,
        preferred_primary_base: Option<usize>,
        preferred_second_space: Option<usize>,
        descriptor_manager: M,
    ) -> Result<UwUGC, CreateError> {
        let (send, recv) = mpsc::channel();
        send.send(MM::new(size, preferred_second_space)?).unwrap();

        let shared = Arc::new(
            GCSync::new(SharedState {
                mm: MM::new(size, preferred_primary_base)?,
                second_mm: Mutex::new(recv),
                second_mm_done: send,
                contexts: Mutex::new(HashMap::new()),
                mark_true_bit: Bit::Bit1,
                type_manager: TypeManagerConcrete::new(descriptor_manager),
            })
            .map_err(|x| x.0)?,
        );
        let controller = Arc::new(GCController::new());

        Ok(UwUGC {
            shared: shared.clone(),
            controller: controller.clone(),
            gc_thread: ManuallyDrop::new(thread::spawn(move || gc_thread(shared, controller))),
        })
    }
}

fn gc_thread(shared: Arc<GCSync<SharedState>>, controller: Arc<GCController>) {
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
        gc_state = Some(gc::do_cycle(&shared, &controller, gc_state.take()));
        println!("[GC] Cycle end");
    });

    println!("[GC] Shutdown");
}
