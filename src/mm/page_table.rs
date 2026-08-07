use std::{ptr::NonNull, sync::atomic::{AtomicUsize, Ordering}};

use parking_lot::Mutex;

use crate::mm::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

pub struct PageTable {
    base_addr: *mut u8,
    current_base_page: AtomicUsize,
    nr_pages: usize,

    // TODO: Turn these to be much more efficient than mutex
    // maybe use some UnsafeCells, AtomicPtr or pointer
    page_table: Vec<Mutex<Option<FlexPage>>>,
    medium_buffer_page: Mutex<Option<usize>>,
}

unsafe impl Sync for PageTable {}
unsafe impl Send for PageTable {}

impl PageTable {
    pub fn new(base_addr: *mut u8, nr_pages: usize) -> Self {
        let mut page_table = Vec::new();
        page_table.resize_with(nr_pages, Default::default);

        Self {
            base_addr,
            nr_pages,
            current_base_page: AtomicUsize::new(0),
            medium_buffer_page: Mutex::new(None),
            page_table,
        }
    }

    pub fn nr_pages(&self) -> usize {
        self.nr_pages
    }

    pub fn clear(&mut self) {
        *self.current_base_page.get_mut() = 0;
        self.page_table.iter_mut().for_each(|x| *x.get_mut() = None);
        *self.medium_buffer_page.get_mut() = None;
    }

    // Whether caller own or not the memory, depends on where you got
    // the base_addr. Second value in tuple. Is page id where its allocated
    // in
    pub fn alloc(&self, size: usize) -> Option<(*mut u8, usize)> {
        assert!(size.is_multiple_of(8), "Object sizes must be multiple of 8");
        if size >= FlexPageKind::Medium.max_object_size() {
            // Huge object, skip over the medium
            let page_id = self.alloc_page(FlexPageKind::Huge {
                base_count: size.div_ceil(BASE_PAGE_SIZE),
            })?;

            return Some((
                self.page_table[page_id]
                    .lock()
                    .as_mut()
                    .unwrap()
                    .alloc(size)
                    .unwrap(),
                page_id,
            ));
        }

        let mut buf_id = self.medium_buffer_page.lock();

        if buf_id.is_none() {
            *buf_id = Some(self.alloc_page(FlexPageKind::Medium)?);
        }

        let mut page = self.page_table[buf_id.unwrap()].lock();
        if page.as_mut().unwrap().free() < size {
            // Not enough space, allocate new FlexPage
            *buf_id = Some(self.alloc_page(FlexPageKind::Medium)?);
            page = self.page_table[buf_id.unwrap()].lock();
        }

        let page = page.as_mut().unwrap();
        assert!(page.free() >= size);

        Some((page.alloc(size).unwrap(), buf_id.unwrap()))
    }

    pub fn get_page<'a>(&'a self, idx: usize) -> &'a Mutex<Option<FlexPage>> {
        &self.page_table[idx]
    }

    pub fn set_base(&mut self, new_base: *mut u8) {
        let old_base = self.base_addr;
        for page in self.page_table.iter_mut() {
            let Some(page) = page.get_mut().as_mut() else {
                continue;
            };
            page.start =
                NonNull::new(new_base.wrapping_byte_add(page.start.addr().get() - old_base.addr()))
                    .unwrap();
        }
        self.base_addr = new_base;
    }

    // Return page index where its allocated. Caller "owns" the range of memory
    // represented. But whether that range is valid to be dereferenced depends
    // on where you got the base_addr pointer from. This PageTable memory keep
    // track pointers.
    pub fn alloc_page(&self, kind: FlexPageKind) -> Option<usize> {
        let page_index = self
            .current_base_page
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |x| {
                let new = x + kind.nr_pages();
                if new > self.nr_pages { None } else { Some(new) }
            })
            .ok()?;

        let mut slot = self.page_table[page_index].lock();
        *slot = Some(FlexPage::new(
            kind,
            self.base_addr
                .wrapping_byte_add(page_index * BASE_PAGE_SIZE),
        ));

        Some(page_index)
    }
}
