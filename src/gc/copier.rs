use std::{slice, sync::atomic::Ordering};

use crate::{
    bitmap::AtomicBitmap,
    gc::{HeapInfoLater, relocation_map::FrozenRegistry},
    mm::{BASE_PAGE_SHIFT, BASE_PAGE_SIZE, FlexPage, PageTable},
    mmap::Mmap,
    object::ObjectPtr,
    type_manager::TypeManagerConcrete,
};

pub struct Copier {}

impl Copier {
    pub fn new() -> Self {
        Self {}
    }

    // note, heap_len may be larger than last byte in compacted form
    // # Safety
    // Caller has to ensure the heap described by 'heap' is not
    // currently used for duration of this function
    pub unsafe fn start(
        self,
        heap: HeapInfoLater,
        reloc_registry: FrozenRegistry,
        from_mapping: Mmap,
        compacted_page_table: PageTable,
        type_manager: &TypeManagerConcrete,
    ) -> CopierActive {
        // Do dumb copying, leaving "later used" unmoved
        // later update to use userfaultfd

        let active = CopierActive {
            state: self,
            reloc_registry,
            work_done: AtomicBitmap::new(compacted_page_table.nr_pages()),
            heap,
            from_mapping,
            zero_page_start: compacted_page_table.get_used_end_page(),
            page_table: compacted_page_table,
        };

        // Pretend we faulted entire space from start of heap to end
        // on each two pages
        for i in 0.. {
            let fault_addr = active.heap.start.wrapping_byte_add(i * 8192);
            if fault_addr >= active.heap.start.wrapping_byte_add(active.heap.size) {
                // Faulted entire heap
                break;
            }

            active.resolve_fault(fault_addr, type_manager);
        }

        active
    }
}

pub struct CopierActive {
    state: Copier,
    reloc_registry: FrozenRegistry,
    heap: HeapInfoLater,
    from_mapping: Mmap,

    // Each index correspond to one page base page processed.
    work_done: AtomicBitmap,

    // Page index where zero paging begins (no relocating necessary
    // just UFFDIO_ZEROPAGE)
    zero_page_start: usize,

    // TODO: Maybe optimize memory bit better to use bitmap? of where
    // is valid start page
    page_table: PageTable,
}

impl CopierActive {
    fn do_zeropage(&self, page_id: usize) {
        if self.work_done.set(page_id, true, Ordering::Relaxed) {
            // Have zeropaged this page. Pretend its spurious page faults
            return;
        }

        let start = self.heap.start.wrapping_byte_add(page_id * BASE_PAGE_SIZE);
        let len = BASE_PAGE_SIZE;

        // SAFETY: Only one thread and one do_relocate that can update a single page
        // via atomic bit map on work_done
        let dest_slice = unsafe { slice::from_raw_parts_mut(start, len) };
        dest_slice.fill(0);
    }

    fn do_relocate(&self, page_id: usize, page: &FlexPage, type_manager: &TypeManagerConcrete) {
        if self.work_done.set(page_id, true, Ordering::Relaxed) {
            // Have relocated to this page. Pretend its spurious page faults
            return;
        }

        let start = page.start();
        let end = start.wrapping_byte_add(page.size());

        let start_offset = start.addr() - self.heap.start.addr();
        let end_offset = end.addr() - self.heap.start.addr();
        let page_range = start_offset..end_offset;

        for record in self
            .reloc_registry
            .iterate_records_in_dest_range(&(start_offset..end_offset))
        {
            // Make sure that record fully in the page. By page design and allocator
            // it has to be fully in page. No object split between 2 pages
            let dest_range = record.get_dest_range();
            assert!(page_range.contains(&dest_range.start));
            assert!(page_range.contains(&(dest_range.end - 1)));

            let src_ptr = self.from_mapping.get_ptr().wrapping_add(record.src);
            let dest = self.heap.start.wrapping_byte_add(record.dest);

            // SAFETY: Already make sure destination is not being written by other
            // it cannot happen because work_done bitmap ensure only one thread/do_relocate
            // can modifies destination
            unsafe {
                std::ptr::copy_nonoverlapping(src_ptr.cast_const(), dest, record.size);
            };

            // Perform pointer fixing
            // SAFETY: Each record in relocation map correspond to one valid object
            // so after copying, the dest always points to object header
            let object = unsafe { ObjectPtr::new(dest) };

            let mut updater = |x: ObjectPtr| -> ObjectPtr {
                let mapped = self
                    .reloc_registry
                    .map_src_to_dest(x.to_ptr().addr() - self.heap.start.addr())
                    .expect("Cant find relocation record");

                // SAFETY: Each record is valid at object boundry
                unsafe { ObjectPtr::new(self.heap.start.wrapping_byte_add(mapped)) }
            };

            // SAFETY: We have exclusive control over destination, work_done
            // bitmap prevent concurrent writes
            unsafe { type_manager.update_gc_pointers(object, &mut updater) };
        }
    }

    fn resolve_fault(&self, addr: *mut u8, type_manager: &TypeManagerConcrete) {
        let page_id = (addr.addr() - self.heap.start.addr()) >> BASE_PAGE_SHIFT;
        if page_id >= self.zero_page_start {
            self.do_zeropage(page_id);
            return;
        }

        let Some(page_id) = self.page_table.resolve_to_page(addr) else {
            self.do_zeropage(page_id);
            return;
        };

        let page = self.page_table.get_page(page_id).lock();
        self.do_relocate(page_id, page.as_ref().unwrap(), type_manager);
    }

    pub fn finish(self) -> (FrozenRegistry, Copier, Mmap) {
        (self.reloc_registry, self.state, self.from_mapping)
    }
}
