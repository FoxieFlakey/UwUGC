// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{ffi::c_void, io, mem, ptr};

mod context;
mod page;
mod page_table;

pub use context::Context;
use nix::errno::Errno;
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
    pub unsafe fn remap_and_clear(
        &mut self,
        target: Option<Mmap>,
        new_table: Option<PageTable>,
    ) -> io::Result<(PageTable, Mmap)> {
        let nr_pages = self.page_table.nr_pages();
        let len = self.page_table.nr_pages() * BASE_PAGE_SIZE;
        if let Some(prev) = target.as_ref() {
            assert!(prev.len() == len, "target mapping does not have same length as current MM");
        }

        let mut flags = nix::libc::MREMAP_DONTUNMAP | nix::libc::MREMAP_MAYMOVE;
        if target.is_some() {
            flags |= nix::libc::MREMAP_FIXED;
        }

        // SAFETY: a
        let moved = Errno::result(unsafe {
            nix::libc::mremap(
                self.mapping.get_ptr().cast(),
                len,
                len,
                flags,
                target.as_ref().map(|x| x.get_ptr()).unwrap_or(ptr::null_mut()).cast::<c_void>(),
            )
        })?
        .cast::<u8>();

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
        moved_page_table.set_base(moved);

        // Forget the target, because we're recreating Mmap unconditionally
        mem::forget(target);
        
        // SAFETY: This mapping can be munmap like normal
        let moved = unsafe { Mmap::from_raw(moved, len) };
        Ok((moved_page_table, moved))
    }
}
