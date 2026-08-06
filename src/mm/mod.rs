// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{
    io, mem,
    ptr::NonNull,
    sync::atomic::{AtomicUsize, Ordering},
};

use memmap2::{MmapMut, RemapOptions};
use parking_lot::Mutex;

mod context;
mod page;

pub use context::Context;
pub use page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

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

    #[expect(unused)]
    pub fn size(&self) -> usize {
        self.mapping.len()
    }

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

    // # Safety
    // Caller also must ensure that all live contexts be flushed by flush_local_buf
    pub unsafe fn remap_and_clear(&mut self) -> io::Result<MM> {
        // SAFETY: We're passing None, so it always move to new
        // safe mapping.
        unsafe { self.remap_and_clear_impl(None) }
    }

    // Remap this MM to other location optionally with target
    //
    // # Safety
    // Caller must make sure if hint is Some, it must not be any mapping
    // If needed to remap into other MM, use remap_into_and_clear.
    //
    // Caller also must ensure that all live contexts be flushed by flush_local_buf
    unsafe fn remap_and_clear_impl(&mut self, target: Option<usize>) -> io::Result<MM> {
        // SAFETY: Caller ensured that target either None (always moves to unused space)
        // or Some which ensures it must not be used by anything
        let moved = unsafe {
            self.mapping
                .move_mapping_and_clear(RemapOptions::new().may_move(true), target)
        }?;
        let mut empty_table = Vec::new();
        empty_table.resize_with(self.nr_pages, Default::default);

        // Clear current table, and take the old table
        // to be moved
        let mut moved_page_table = mem::replace(&mut self.page_table, empty_table);

        // Fix the pointer in page table
        let old_base = self.mapping.ptr_mut();
        let new_base = moved.ptr_mut();
        for page in moved_page_table.iter_mut() {
            let Some(page) = page.get_mut().as_mut() else {
                continue;
            };
            page.start =
                NonNull::new(new_base.wrapping_byte_add(page.start.addr().get() - old_base.addr()))
                    .unwrap();
        }

        // New MM describing the moved space
        let moved_mm = MM {
            page_table: moved_page_table,
            current_base_page: mem::replace(&mut self.current_base_page, AtomicUsize::new(0)),
            mapping: moved,
            medium_buffer_page: mem::replace(&mut self.medium_buffer_page, Mutex::new(None)),
            nr_pages: self.nr_pages,
        };

        Ok(moved_mm)
    }
}
