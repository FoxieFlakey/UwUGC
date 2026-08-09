// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{io, mem};

mod context;
mod page;
mod page_table;

pub use context::Context;
pub use page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

pub use page_table::PageTable;

use crate::mmap::Mmap;

pub struct MM {
    mapping: Mmap,
    page_table: PageTable,
}

unsafe impl Send for MM {}
unsafe impl Sync for MM {}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("Cannot create heap")]
    MmapError(#[from] io::Error),
}

impl MM {
    pub fn new(size: usize) -> Result<Self, CreateError> {
        let nr_pages = size.div_ceil(BASE_PAGE_SIZE);
        let mapping = Mmap::map(nr_pages * BASE_PAGE_SIZE, true, true, true)?;
        Ok(Self {
            page_table: PageTable::new(mapping.get_ptr(), nr_pages),
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
    // replace the page table. Caller also can remap into existing mapping
    //
    // # Safety
    // Caller also must ensure that all live contexts be flushed by flush_local_buf
    //
    // If new_page_table is Some, all previous allocation by other part
    // may or may not become "invalid" depends on what the new table said.
    pub unsafe fn remap(
        &mut self,
        target: &mut Option<Mmap>,
        new_table: Option<PageTable>,
    ) -> io::Result<(PageTable, Mmap)> {
        let nr_pages = self.page_table.nr_pages();
        let len = self.page_table.nr_pages() * BASE_PAGE_SIZE;
        if let Some(prev) = target.as_ref() {
            assert!(
                prev.len() == len,
                "target mapping does not have same length as current MM"
            );
        }

        // SAFETY: Caller ensured nothing uses the current mapping
        let remapped = unsafe { self.mapping.remap(target) }?;
        let new_table = new_table
            .map(|mut x| {
                assert_eq!(
                    x.nr_pages(),
                    nr_pages,
                    "New page table doesnt manage same memory size as current MM"
                );
                x.set_base(self.mapping.get_ptr());
                x
            })
            .unwrap_or_else(|| PageTable::new(self.mapping.get_ptr(), nr_pages));
        let mut moved_page_table = mem::replace(&mut self.page_table, new_table);

        // Fix the pointer in page table
        moved_page_table.set_base(remapped.get_ptr());

        // SAFETY: This mapping can be munmap like normal
        Ok((moved_page_table, remapped))
    }
}
