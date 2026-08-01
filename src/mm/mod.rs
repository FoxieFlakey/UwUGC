// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{
    io,
    sync::atomic::{AtomicUsize, Ordering},
};

use memmap2::MmapMut;
use parking_lot::Mutex;

use crate::mm::page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

mod context;
mod page;

pub use context::Context;

pub struct MM {
    mapping: MmapMut,
    current_base_page: AtomicUsize,
    nr_pages: usize,

    // TODO: Turn these to be much more efficient than mutex
    // maybe use some UnsafeCells, AtomicPtr or pointer
    page_table: Vec<Mutex<Option<FlexPage>>>,
    medium_buffer_page: Mutex<Option<usize>>,
}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("Cannot create heap")]
    MmapError(#[from] io::Error),
}

impl MM {
    pub fn new(size: usize) -> Result<Self, CreateError> {
        let nr_pages = size.div_ceil(BASE_PAGE_SIZE);
        let mut page_table = Vec::new();
        page_table.resize_with(nr_pages, Default::default);

        Ok(Self {
            current_base_page: AtomicUsize::new(0),
            nr_pages,
            mapping: MmapMut::map_anon(nr_pages * BASE_PAGE_SIZE)?,
            page_table,
            medium_buffer_page: Mutex::new(None),
        })
    }

    pub fn size(&self) -> usize {
        self.mapping.len()
    }

    pub fn alloc(&self, size: usize) -> Option<*mut u8> {
        if size >= FlexPageKind::Medium.max_object_size() {
            // Huge object, skip over the medium
            let page_id = self.alloc_page(FlexPageKind::Huge {
                base_count: size.div_ceil(BASE_PAGE_SIZE),
            })?;

            return Some(
                self.page_table[page_id]
                    .lock()
                    .as_mut()
                    .unwrap()
                    .alloc(size)
                    .unwrap(),
            );
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

        Some(page.alloc(size).unwrap())
    }

    // Return page index where its allocated. Caller owns the range of memory
    // represented
    pub(self) fn alloc_page(&self, kind: FlexPageKind) -> Option<usize> {
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
            self.mapping
                .ptr_mut()
                .wrapping_byte_add(page_index * BASE_PAGE_SIZE),
        ));

        Some(page_index)
    }
}
