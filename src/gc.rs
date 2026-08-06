use std::sync::Arc;

use crate::{gc_controller::GCController, gc_sync::GCSync, state::SharedState};

pub struct PersistentState {}

pub fn do_cycle(shared: &Arc<GCSync<SharedState>>, controller: &Arc<GCController>) {
    let mut heap = shared.get_exclusive();
    let gc = heap.get().gc_state.take().unwrap_or_else(|| {
        // GC may store persistent state like caching few stuffs
        // or store reusable stuffs to avoid reallocating on each cycle
        PersistentState {}
    });

    // Take snapshot of root set (a.k.a the SATB)
    let saved_roots = heap
        .get()
        .contexts
        .get_mut()
        .iter()
        .map(|(_, v)| {
            let guard = v.lock();

            // SAFETY: We're in STW nothing is modifying the root set at all
            let raw_cloned = unsafe { guard.root_set.get_raw().clone() };

            guard.root_set.clone_metadata(raw_cloned)
        })
        .collect::<Vec<_>>();

    // Doesnt care any start_gc that occur during first phase
    controller.clear_request();
    drop(heap);

    // Let pretend we marked the heap
    drop(saved_roots);

    let mut heap = shared.get_exclusive();
    // SAFETY: For now, we assume all objects are dead
    let _ = unsafe { heap.get().mm.remap_and_clear() }.unwrap();
    
    heap.get()
        .gc_state
        .set(gc)
        .ok()
        .expect("GC persistent state somehow is initialized?");
    heap.get().contexts.get_mut().iter().for_each(|x| {
        let root_set = &x.1.lock().root_set;
        unsafe {
            root_set.map_pointers(&mut |x| {
                // Lets assume we modifies or fixed the pointer :3
                x
            });
        };
    });
}
