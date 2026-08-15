use std::{os::fd::AsFd, sync::atomic::Ordering};

use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use rayon::{ThreadPool, ThreadPoolBuilder};
use userfaultfd::{Uffd, UffdBuilder};

use crate::{
    bitmap::AtomicBitmap, gc::{HeapInfoLater, relocation_map::FrozenRegistry}, mm::{BASE_PAGE_SHIFT, BASE_PAGE_SIZE, FlexPage, MM, PageTable}, mmap::Mmap, object::ObjectPtr, pipe::Pipe, quirks, type_manager::TypeManagerConcrete
};

pub struct Copier {
    uffd: Uffd,
    worker_pool: ThreadPool,
    uffd_pool: ThreadPool,
    pipe: Pipe<u8>,
}

impl Copier {
    pub fn new() -> Self {
        Self {
            worker_pool: ThreadPoolBuilder::new()
                .num_threads(4)
                .thread_name(|x| format!("Worker-{x:02}"))
                .build()
                .unwrap(),
            uffd_pool: ThreadPoolBuilder::new()
                .num_threads(4)
                .thread_name(|x| format!("UFFD-{x:02}"))
                .build()
                .unwrap(),
            uffd: UffdBuilder::new()
                .close_on_exec(true)
                .non_blocking(true)
                .user_mode_only(true)
                .create()
                .unwrap(),
            pipe: Pipe::new().unwrap(),
        }
    }

    // note, heap_len may be larger than last byte in compacted form
    // # Safety
    // Caller has to ensure the heap described by 'heap' is not
    // currently used for duration of this function
    pub unsafe fn start(
        self,
        heap: HeapInfoLater,
        reloc_registry: FrozenRegistry,
        from_mm: MM,
        compacted_page_table: PageTable,
        _type_manager: &TypeManagerConcrete,
    ) -> CopierActive {
        // Activate UFFD on to-space
        self.uffd.register(heap.to_space.cast(), heap.size).unwrap();

        CopierActive {
            state: self,
            reloc_registry,
            work_done: AtomicBitmap::new(compacted_page_table.nr_pages()),
            heap,
            from_mm,
            zero_page_start: compacted_page_table.get_used_end_page(),
            page_table: compacted_page_table,
        }
    }
}

pub struct CopierActive {
    state: Copier,
    reloc_registry: FrozenRegistry,
    heap: HeapInfoLater,
    from_mm: MM,

    // Each index correspond to one page base page processed.
    work_done: AtomicBitmap,

    // Page index where zero paging begins (no relocating necessary
    // just UFFDIO_ZEROPAGE)
    zero_page_start: usize,

    // PageTable here is in from-space not to-space
    // TODO: Maybe optimize memory bit better to use bitmap? of where
    // is valid start page
    page_table: PageTable,
}

unsafe impl Sync for CopierActive {}

impl CopierActive {
    fn do_zeropage(&self, page_id: usize) {
        if self.work_done.set(page_id, true, Ordering::Relaxed) {
            // Have zeropaged this page. Pretend its spurious page faults
            return;
        }

        let start = (self.page_table.get_base_addr() as *mut u8).wrapping_byte_add(page_id * BASE_PAGE_SIZE);
        let len = BASE_PAGE_SIZE;

        unsafe { self.state.uffd.zeropage(start.cast(), len, true) }.unwrap();
    }

    fn do_relocate(&self, page_id: usize, page: &FlexPage, type_manager: &TypeManagerConcrete) {
        if self.work_done.set(page_id, true, Ordering::Relaxed) {
            // Have relocated to this page. Pretend its spurious page faults
            return;
        }

        let buffer = Mmap::map(page.size(), true, true, true, None).unwrap();

        let start = page.start();
        let end = start.wrapping_byte_add(page.size());

        let start_offset = start.addr() - self.heap.to_space.addr();
        let end_offset = end.addr() - self.heap.to_space.addr();
        let page_range = start_offset..end_offset;

        let dest_page_offset = start.addr() - self.heap.to_space.addr();

        for record in self
            .reloc_registry
            .iterate_records_in_dest_range(&(start_offset..end_offset))
        {
            // Make sure that record fully in the page. By page design and allocator
            // it has to be fully in page. No object split between 2 pages
            let dest_range = record.get_dest_range();
            assert!(page_range.contains(&dest_range.start));
            assert!(page_range.contains(&(dest_range.end - 1)));

            let src_ptr = self
                .from_mm
                .get_mapping()
                .get_ptr()
                .wrapping_add(record.src);
            let dest = buffer
                .get_ptr()
                .wrapping_byte_add(record.dest - dest_page_offset);

            // SAFETY: Already make sure destination is not being written by other
            // it cannot happen because work_done bitmap ensure only one thread/do_relocate
            // can modifies destination
            unsafe {
                std::ptr::copy_nonoverlapping(src_ptr.cast_const(), dest, record.size);
            };

            // Perform pointer fixing
            // SAFETY: Each record in relocation map correspond to one valid object
            // so after copying, the dest always points to object header
            let object = unsafe { ObjectPtr::from_raw(dest) }.unwrap();

            let mut updater = |x: ObjectPtr| -> ObjectPtr {
                let mapped = self
                    .reloc_registry
                    .map_src_to_dest(x.into_raw().addr().get() - self.heap.start.addr())
                    .expect("Cant find relocation record");

                // SAFETY: Each record is valid at object boundry
                unsafe { ObjectPtr::from_raw(self.heap.to_space.wrapping_byte_add(mapped)) }
                    .unwrap()
            };

            // SAFETY: We have exclusive control over destination, work_done
            // bitmap prevent concurrent writes
            unsafe { type_manager.update_gc_pointers(object, &mut updater) };
        }

        // Then finally move to final via uffd move
        let mut src = buffer.get_ptr().cast();
        let mut dest = self
            .heap
            .to_space
            .wrapping_byte_add(page.start().addr() - self.page_table.get_base_addr())
            .cast();
        let mut len = page.size();

        loop {
            match unsafe { self.state.uffd.move_memory(src, dest, len, true, true) } {
                Ok(moved) => {
                    assert_eq!(len, moved, "short move is not returned as partially move?");
                    break;
                }

                Err(userfaultfd::Error::PartiallyCopied(mut moved)) => {
                    quirks::uffd_try_fix_moved(dest, &mut moved, len);
                    if moved == len {
                        // All pages actually moved. But kernel under-reporting
                        break;
                    }

                    len -= moved;
                    src = src.wrapping_byte_add(moved);
                    dest = dest.wrapping_byte_add(moved);
                }

                Err(e) => {
                    panic!("Cannot move pages: {e}");
                }
            }
        }
    }

    fn resolve_fault(&self, addr: *mut u8, type_manager: &TypeManagerConcrete) {
        let page_id = (addr.addr() - self.heap.to_space.addr()) >> BASE_PAGE_SHIFT;
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

    pub fn finish(self, type_manager: &TypeManagerConcrete) -> (FrozenRegistry, Copier, MM) {
        struct SyncPtr(*mut u8);
        unsafe impl Send for SyncPtr {}

        self.state.worker_pool.in_place_scope_fifo(|s| {
            // Spawn job that eagerly tries to relocate
            s.spawn_fifo(|s| {
                for id in 0..self.page_table.nr_pages() {
                    let page = self.page_table.get_page(id).lock();
                    if page.is_none() {
                        continue;
                    }

                    let addr = SyncPtr(page.as_ref().unwrap().start());
                    let self_borrow = &self;
                    s.spawn_fifo(move |_| {
                        let addr = addr;
                        self_borrow.resolve_fault(addr.0, type_manager);
                    });
                }

                // Pipe isnt coped to handle ZST types currently use u8
                // indicates that all pages is relocated
                self.state.pipe.write(0).unwrap();
            });

            // Userfaultfd handling loop
            self.state.uffd_pool.in_place_scope_fifo(|s| {
                loop {
                    let mut pollfd = [
                        PollFd::new(self.state.uffd.as_fd(), PollFlags::POLLIN),
                        PollFd::new(self.state.pipe.get_read_fd(), PollFlags::POLLIN),
                    ];

                    poll(&mut pollfd, PollTimeout::NONE).unwrap();

                    if !pollfd[0].revents().unwrap().is_empty() {
                        if let Some(event) = self.state.uffd.read_event().unwrap() {
                            match event {
                                userfaultfd::Event::Pagefault {
                                    kind: userfaultfd::FaultKind::Missing,
                                    addr,
                                    ..
                                } => {
                                    // Dispatch userfaultfd handling
                                    let addr = SyncPtr(addr.cast());
                                    let self_borrow = &self;
                                    s.spawn_fifo(move |_| {
                                        let addr = addr;
                                        self_borrow.resolve_fault(addr.0, type_manager)
                                    });
                                }

                                _ => unimplemented!(),
                            }
                        }
                    }

                    if !pollfd[1].revents().unwrap().is_empty() {
                        // Bulk updating threads are done, lets finish
                        // copying
                        self.state.pipe.read().unwrap();
                        break;
                    }
                }
            });

            // While there UFFD event lets exhaust them
            // dont act on it because worker already used all of them up
            while let Some(_) = self.state.uffd.read_event().unwrap() {}
        });

        // Deactivate UFFD
        self.state
            .uffd
            .unregister(self.heap.to_space.cast(), self.heap.size)
            .unwrap();
        (self.reloc_registry, self.state, self.from_mm)
    }
}
