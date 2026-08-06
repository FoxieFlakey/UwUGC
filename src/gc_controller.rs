use parking_lot::{Condvar, Mutex};

#[derive(Clone)]
struct State {
    is_running: bool,
    is_requested: bool,
    is_shutting_down: bool,
    finished_count: u64,
}

pub struct GCController {
    state: Mutex<State>,
    condvar: Condvar,
    cycle_done_condvar: Condvar,
}

impl GCController {
    pub fn new() -> GCController {
        GCController {
            condvar: Condvar::new(),
            cycle_done_condvar: Condvar::new(),
            state: Mutex::new(State {
                is_running: false,
                is_shutting_down: false,
                is_requested: false,
                finished_count: 0,
            }),
        }
    }

    pub fn shutdown(&self) {
        self.state.lock().is_shutting_down = true;
        self.condvar.notify_all();
    }

    #[expect(unused)]
    pub fn start_cycle(&self) {
        self.state.lock().is_requested = true;
        self.condvar.notify_all();
    }

    pub fn start_and_wait_cycle(&self) {
        let mut state = self.state.lock();
        let cur_count = state.finished_count;
        state.is_requested = true;

        // Wait while a cycle is running
        self.cycle_done_condvar
            .wait_while(&mut state, |x| cur_count == x.finished_count);
    }

    // NOTE: careful with this. start gc might get erased
    // by accident if its not first STW (normally capture
    // heap snapshot). Because start_gc might called multiple
    // times while GC waits for threads to park. And some of threads
    // might call it too. When cycle is running its first STW.
    // start_gc can be combined into one as they're looking
    // same memory state.
    pub fn clear_request(&self) {
        self.state.lock().is_requested = false;
    }

    pub fn do_looper(&self, mut on_cycle: impl FnMut()) {
        let mut state = self.state.lock();
        while !state.is_shutting_down {
            self.condvar.wait_while(&mut state, |x| !x.is_requested);
            state.is_running = true;
            state.is_requested = false;
            drop(state);

            on_cycle();

            state = self.state.lock();
            state.finished_count += 1;
            self.cycle_done_condvar.notify_all();
        }
    }
}
