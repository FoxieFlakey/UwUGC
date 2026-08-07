// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{io, mem};

use memmap2::{MmapMut, RemapOptions};

mod context;
mod page;
mod page_table;

pub use context::Context;
pub use page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

pub use page_table::PageTable;

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

    // Remap this MM to other location optionally with target and optionally
    // replace the page table
    //
    // # Safety
    // Caller must make sure if hint is Some, it must not be any mapping
    // If needed to remap into other MM, use remap_into_and_clear.
    //
    // Caller also must ensure that all live contexts be flushed by flush_local_buf
    //
    // If new_page_table is Some, all previous allocation by other part
    // may or may not become "invalid" depends on what the new table said.
    pub unsafe fn remap_and_clear(&mut self, target: Option<usize>, new_table: Option<PageTable>) -> io::Result<(PageTable, MmapMut)> {
        let nr_pages = self.page_table.nr_pages();
        
        // SAFETY: Caller ensured that target either None (always moves to unused space)
        // or Some which ensures it must not be used by anything
        let moved = unsafe {
            self.mapping
                .move_mapping_and_clear(RemapOptions::new().may_move(true), target)
        }?;

        let new_table = new_table.map(|mut x| {
                assert_eq!(x.nr_pages(), nr_pages, "New page table doesnt manage same memory size as current MM");
                x.set_base(self.mapping.ptr_mut());
                x
            }).unwrap_or_else(|| PageTable::new(self.mapping.ptr_mut(), nr_pages));
        let mut moved_page_table = mem::replace(&mut self.page_table, new_table);

        // Fix the pointer in page table
        moved_page_table.set_base(moved.ptr_mut());

        Ok((moved_page_table, moved))
    }
}
