use std::sync::atomic::Ordering;

use humansize::{BINARY, FormatSize};

use crate::{
    gc::{
        HeapInfo,
        relocation_map::{RegistryBuilder, RelocationRecord},
    },
    mm::{Context, PageTable},
    object::{Bit, ObjectPtr},
    root_set::RootSet,
    state::SharedState,
};

pub struct Marker {}

impl Marker {
    pub fn new() -> Self {
        Self {}
    }

    pub fn start(
        self,
        roots: Vec<Box<dyn RootSet + 'static>>,
        mut page_table: PageTable,
        mut registry: RegistryBuilder,
        heap: &SharedState,
        heap_info: &HeapInfo,
        mark_true_bit: Bit,
    ) -> (Self, PageTable, RegistryBuilder) {
        page_table.clear();
        page_table.set_base(heap_info.start);
        let mut move_context = Context::new();

        // Mark objects concurrently, note for now the mark bit doesnt
        // get used its assume all dead
        let mut live_count = 0;
        let mut total_count = 0;

        let type_manager = &heap.offset_walker;
        let mut visitor = |obj: &ObjectPtr| {
            // Mark the object
            let ret =
                obj.metadata_ref()
                    .try_update(Ordering::Relaxed, Ordering::Relaxed, |mut x| {
                        if x.is_marked == mark_true_bit {
                            None
                        } else {
                            x.is_marked = mark_true_bit;
                            Some(x)
                        }
                    });

            total_count += 1;
            if ret.is_ok() {
                // This just first marked.
                // TODO: push object to mark stack to be continued
                // recusrively
                live_count += 1;

                let size = type_manager.get_size(obj);

                // SAFETY: We use same page table consistently
                let dest = unsafe { move_context.alloc_from_page_table(&page_table, size) }
                    .unwrap()
                    .0
                    .addr();

                // Registry only contains offsets
                registry.insert(RelocationRecord {
                    src: obj.to_ptr().addr() - heap_info.start.addr(),
                    dest: dest - heap_info.start.addr(),
                    size,
                });
            } else {
                // Already marked this, either a while ago, or another thread
            }
        };

        for root in roots {
            root.iter_pointers(&mut visitor);
        }

        let used = heap_info.used_end.addr() - heap_info.start.addr();
        let compacted = page_table.get_top_addr() - heap_info.start.addr();
        println!("[GC] Live count: {:9}", live_count);
        println!(
            "[GC] Compacted from {:10} to {:10}",
            used.format_size(BINARY),
            compacted.format_size(BINARY)
        );

        (self, page_table, registry)
    }
}
