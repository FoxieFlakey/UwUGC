use std::sync::Arc;

use crate::{gc_controller::GCController, gc_sync::GCSync, state::SharedState};

pub fn do_cycle(shared: &Arc<GCSync<SharedState>>, controller: &Arc<GCController>) {
    let heap = shared.get_exclusive();

    // TODO: Actually does something, like capturing roots

    // Doesnt care any start_gc that occur during first phase
    controller.clear_request();
    drop(heap);

    // SAFETY: For now, we assume all objects are dead
    let _ = unsafe { shared.get_exclusive().get().mm.remap_and_clear() }.unwrap();
}
