use std::sync::{Arc, atomic::Ordering};

use crate::{
    gc::relocation_map::{RegistryBuilder, RelocationRecord},
    gc_controller::GCController,
    gc_sync::GCSync,
    mm::{BASE_PAGE_SIZE, Context, PageTable},
    mmap::Mmap,
    object::{MetadataCompressed, ObjectPtr},
    state::SharedState,
};

mod relocation_map;

pub struct PersistentState {
    prev_page_and_temp_mapping: Option<(PageTable, Mmap)>,
    registry: Option<RegistryBuilder>,
}

pub fn do_cycle(shared: &Arc<GCSync<SharedState>>, controller: &Arc<GCController>) {
    let mut heap = shared.get_exclusive();
    let mut gc = heap.get().gc_state.take().unwrap_or_else(|| {
        // GC may store persistent state like caching few stuffs
        // or store reusable stuffs to avoid reallocating on each cycle
        PersistentState {
            prev_page_and_temp_mapping: None,
            registry: Some(RegistryBuilder::new()),
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

    let mapping = heap.get().mm.get_mapping();
    let (mut page_table, mut prev_mapping) = gc
        .prev_page_and_temp_mapping
        .take()
        .map(|(x, y)| (x, Some(y)))
        .unwrap_or((
            PageTable::new(mapping.get_ptr(), mapping.len() / BASE_PAGE_SIZE),
            None,
        ));
    page_table.set_base(mapping.get_ptr());

    let heap_start = mapping.get_ptr().addr();
    let before_gc_heap_end = heap.get().mm.get_page_table().get_top_addr();
    drop(heap);

    let mut registry = gc.registry.take().unwrap();
    let mut move_context = Context::new();

    // Mark objects concurrently, note for now the mark bit doesnt
    // get used its assume all dead
    let mut live_count = 0;
    let mut total_count = 0;
    let mut visitor = |obj: &ObjectPtr| {
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

            let size = obj.size() + size_of::<MetadataCompressed>();

            // SAFETY: We use same page table consistently
            let dest = unsafe { move_context.alloc_from_page_table(&page_table, size) }
                .unwrap()
                .0
                .addr();
            registry.insert(RelocationRecord {
                src: obj.to_ptr().addr(),
                dest,
                size,
            });
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
    let after_gc_heap_end = page_table.get_top_addr();
    let before_size_mib = (before_gc_heap_end - heap_start) as f32 / 1024.0 / 1024.0;
    let after_size_mib = (after_gc_heap_end - heap_start) as f32 / 1024.0 / 1024.0;
    println!(
        "Compacted to 0x{heap_start:016x}..0x{after_gc_heap_end:016x} ({after_size_mib:6.2} MiB) from 0x{heap_start:016x}..0x{before_gc_heap_end:016x} ({before_size_mib:6.2} MiB)"
    );
    let registry_frozen = registry.freeze();

    let mut heap = shared.get_exclusive();

    // SAFETY: For now, we assume all objects are dead
    let mut page_table = Some(page_table);
    let (mut page_table, mapping) =
        unsafe { heap.get().mm.remap(&mut prev_mapping, &mut page_table) }.unwrap();

    // Empty the table for later use by next cycle
    page_table.clear();
    gc.prev_page_and_temp_mapping = Some((page_table, mapping));
    gc.registry = Some(registry_frozen.unfreeze());

    heap.get()
        .gc_state
        .set(gc)
        .ok()
        .expect("GC persistent state somehow is initialized?");
    heap.get().contexts.get_mut().iter().for_each(|x| {
        let root_set = &x.1.lock().root_set;

        // SAFETY: We're in STW so no mutator is running
        unsafe {
            root_set.map_pointers(&mut |x| {
                // Lets assume we modifies or fixed the pointer :3
                x
            });
        };
    });
}
