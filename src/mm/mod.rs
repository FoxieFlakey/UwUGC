// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{io, mem, ptr::NonNull};

use memmap2::{MmapMut, RemapOptions};

mod context;
mod page;
mod page_table;

pub use context::Context;
pub use page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

use crate::mm::page_table::PageTable;

pub struct MM {
    mapping: MmapMut,
    page_table: PageTable,
}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("Cannot create heap")]
    MmapError(#[from] io::Error),
}

impl MM {
    pub fn new(size: usize) -> Result<Self, CreateError> {
        let nr_pages = size.div_ceil(BASE_PAGE_SIZE);
        let mapping = MmapMut::map_anon(nr_pages * BASE_PAGE_SIZE)?;
        Ok(Self {
            page_table: PageTable::new(mapping.ptr_mut(), nr_pages),
            mapping,
        })
    }

    pub fn alloc(&self, size: usize) -> Option<(*mut u8, usize)> {
        // Note: because we own the memory and page table do not
        // cause overlaps. The pointer can be used directly
        self.page_table.alloc(size)
    }

    pub fn get_page_table(&self) -> &PageTable {
        &self.page_table
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

        // Clear current table, and take the old table
        // to be moved
        let empty_table = PageTable::new(self.mapping.ptr_mut(), self.page_table.nr_pages());
        let mut moved_page_table = mem::replace(&mut self.page_table, empty_table);

        // Fix the pointer in page table
        let old_base = self.mapping.ptr_mut();
        let new_base = moved.ptr_mut();
        for page in moved_page_table.page_table.iter_mut() {
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
            mapping: moved,
        };

        Ok(moved_mm)
    }
}
