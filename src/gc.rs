use std::sync::{Arc, atomic::Ordering};

use crate::{
    gc_controller::GCController, gc_sync::GCSync, mm::PageTable, mmap::Mmap, object::ObjectPtr,
    state::SharedState,
};

pub struct PersistentState {
    prev_page_and_temp_mapping: Option<(PageTable, Mmap)>,
}

pub fn do_cycle(shared: &Arc<GCSync<SharedState>>, controller: &Arc<GCController>) {
    let mut heap = shared.get_exclusive();
    let mut gc = heap.get().gc_state.take().unwrap_or_else(|| {
        // GC may store persistent state like caching few stuffs
        // or store reusable stuffs to avoid reallocating on each cycle
        PersistentState {
            prev_page_and_temp_mapping: None,
        }
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

    // Mark objects concurrently, note for now the mark bit doesnt
    // get used its assume all dead
    let mut live_count = 0;
    let mut total_count = 0;
    let mut visitor = |x| {
        // SAFETY: The implementer of iter_pointers ensures the
        // only pointers that given is the same one GC gave
        let obj = unsafe { ObjectPtr::new(x) };

        // Mark the object
        let ret = obj
            .metadata_ref()
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |mut x| {
                if x.is_marked {
                    None
                } else {
                    x.is_marked = true;
                    Some(x)
                }
            });

        total_count += 1;
        if ret.is_ok() {
            // This just first marked.
            // TODO: push object to mark stack to be continued
            // recusrively
            live_count += 1;
        } else {
            // Already marked this, either a while ago, or another thread
        }
    };

    for root in saved_roots {
        // SAFETY: We cloned the RootSet, so nobody
        // accesses it
        unsafe { root.iter_pointers(&mut visitor) };
    }
    println!("Live count: {live_count}, Total count: {total_count}");

    let mut heap = shared.get_exclusive();

    // SAFETY: For now, we assume all objects are dead
    let (prev_page_table, mut prev_mapping) = gc
        .prev_page_and_temp_mapping
        .take()
        .map(|(x, y)| (Some(x), Some(y)))
        .unwrap_or((None, None));

    let (mut page_table, mapping) = unsafe {
        heap.get()
            .mm
            .remap(&mut prev_mapping, prev_page_table)
    }
    .unwrap();

    // Empty the table for later use by next cycle
    page_table.clear();
    gc.prev_page_and_temp_mapping = Some((page_table, mapping));

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
