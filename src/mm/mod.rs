// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::io;

mod context;
mod page;
mod page_table;

pub use context::Context;
pub use page::{BASE_PAGE_SHIFT, BASE_PAGE_SIZE, FlexPage, FlexPageKind};

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
    pub fn new(size: usize, preferred_heap_base: Option<usize>) -> Result<Self, CreateError> {
        let nr_pages = size.div_ceil(BASE_PAGE_SIZE);
        let mapping = Mmap::map(
            nr_pages * BASE_PAGE_SIZE,
            true,
            true,
            true,
            preferred_heap_base,
        )?;
        Ok(Self {
            page_table: PageTable::new(mapping.get_ptr(), nr_pages),
            mapping,
        })
    }

    pub fn get_page_table_cloned(&mut self) -> PageTable {
        self.page_table.clone()
    }

    pub fn get_page_table_mut(&mut self) -> &mut PageTable {
        &mut self.page_table
    }

    pub fn get_page_table(&self) -> &PageTable {
        &self.page_table
    }

    pub fn get_mapping(&self) -> &Mmap {
        &self.mapping
    }

    pub fn clear(&mut self) {
        // SAFETY: &mut ensures that nothing accesss the backing memory anymore
        unsafe { self.mapping.advise(crate::mmap::Advice::DontNeed).unwrap() };
    }
}
