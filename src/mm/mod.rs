// This uses mechanism like ZGC's ZPage and ZPageTable. Using similar parameter
// like small is 2 MiB, medium kinda changing, huge is whatever multiple of 2 MiB

use std::{ffi::c_void, io, mem, ptr};

use memmap2::MmapMut;

mod context;
mod page;
mod page_table;

pub use context::Context;
use nix::errno::Errno;
pub use page::{BASE_PAGE_SIZE, FlexPage, FlexPageKind};

pub use page_table::PageTable;

pub struct MM {
    #[expect(unused)]
    mapping: MmapMut,
    mapping_ptr: *mut u8,
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
        let mut mapping = MmapMut::map_anon(nr_pages * BASE_PAGE_SIZE)?;
        Ok(Self {
            page_table: PageTable::new(mapping.as_mut_ptr(), nr_pages),
            mapping_ptr: mapping.as_mut_ptr(),
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
    // replace the page table. The caler now has to do munmap on the target
    // or returned mapping
    //
    // # Safety
    // Caller make sure 'target' is either unused, or used but "compatible" depending
    // on what caller want to use it with
    //
    // Caller also must ensure that all live contexts be flushed by flush_local_buf
    //
    // If new_page_table is Some, all previous allocation by other part
    // may or may not become "invalid" depends on what the new table said.
    pub unsafe fn remap_and_clear(
        &mut self,
        target: Option<*mut u8>,
        new_table: Option<PageTable>,
    ) -> io::Result<(PageTable, *mut u8)> {
        let nr_pages = self.page_table.nr_pages();
        let len = self.page_table.nr_pages() * BASE_PAGE_SIZE;
        // SAFETY: a
        let moved = Errno::result(unsafe {
            nix::libc::mremap(
                self.mapping_ptr.cast(),
                len,
                len,
                nix::libc::MREMAP_DONTUNMAP | nix::libc::MREMAP_FIXED | nix::libc::MREMAP_MAYMOVE,
                target.unwrap_or(ptr::null_mut()).cast::<c_void>(),
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
                x.set_base(self.mapping_ptr);
                x
            })
            .unwrap_or_else(|| PageTable::new(self.mapping_ptr, nr_pages));
        let mut moved_page_table = mem::replace(&mut self.page_table, new_table);

        // Fix the pointer in page table
        moved_page_table.set_base(moved);

        Ok((moved_page_table, moved))
    }
}
