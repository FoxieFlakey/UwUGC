use crate::mm::{MM, PageTable, page::FlexPageKind};

// Context do not remember the MM stuct
// it originates from. Its up to caller
// to make sure its correct.
pub struct Context {
    local_buffer_page: Option<usize>,
}

impl Context {
    pub fn new() -> Self {
        Self {
            local_buffer_page: None,
        }
    }

    pub fn flush_local_buf(&mut self) {
        self.local_buffer_page = None;
    }

    // Return the pointer to allocation, and page id. where its allocated from. It is
    // safe to intermix alloc and alloc_from_page_table.
    //
    // # Safety
    // like the alloc, caller has to make sure page table,
    // is same one used consistently like before
    pub unsafe fn alloc_from_page_table(
        &mut self,
        page_table: &PageTable,
        size: usize,
    ) -> Option<(*mut u8, usize)> {
        assert!(size.is_multiple_of(8), "Object sizes must be multiple of 8");
        if size >= FlexPageKind::Small.max_object_size() {
            return page_table.alloc(size);
        }

        // Fast path with local buf
        if self.local_buffer_page.is_none() {
            self.local_buffer_page = Some(page_table.alloc_page(FlexPageKind::Small)?);
        }

        let mut local_page = page_table.get_page(self.local_buffer_page.unwrap()).lock();
        if size >= local_page.as_mut().unwrap().free() {
            // Not enough space, allocate new FlexPage
            self.local_buffer_page = Some(page_table.alloc_page(FlexPageKind::Small)?);
            local_page = page_table.get_page(self.local_buffer_page.unwrap()).lock();
        }

        Some((
            local_page.as_mut().unwrap().alloc(size).unwrap(),
            self.local_buffer_page.unwrap(),
        ))
    }

    // Return the pointer to allocation, and page id. where its allocated from. It is
    // safe to intermix alloc and alloc_from_page_table
    //
    // # Safety
    // Caller has to make sure MM, is same one used consistently like before
    pub unsafe fn alloc(&mut self, mm: &MM, size: usize) -> Option<(*mut u8, usize)> {
        // SAFETY: Caller
        unsafe { self.alloc_from_page_table(&mm.page_table, size) }
    }
}
