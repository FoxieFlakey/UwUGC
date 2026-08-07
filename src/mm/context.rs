use crate::mm::{MM, page::FlexPageKind};

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

    // Return the pointer to allocation, and page id. where its allocated from
    pub fn alloc(&mut self, mm: &MM, size: usize) -> Option<(*mut u8, usize)> {
        assert!(size.is_multiple_of(8), "Object sizes must be multiple of 8");
        if size >= FlexPageKind::Small.max_object_size() {
            return mm.alloc(size);
        }

        let page_table = mm.get_page_table();
        // Fast path with local buf
        if self.local_buffer_page.is_none() {
            self.local_buffer_page = Some(page_table.alloc_page(FlexPageKind::Small)?);
        }

        let mut local_page = page_table.get_page(self.local_buffer_page.unwrap()).lock();
        if size >= local_page.as_mut().unwrap().free() {
            // Not enough space, allocate new FlexPage
            self.local_buffer_page = Some(page_table.alloc_page(FlexPageKind::Small)?);
            local_page = mm
                .page_table
                .get_page(self.local_buffer_page.unwrap())
                .lock();
        }

        Some((
            local_page.as_mut().unwrap().alloc(size).unwrap(),
            self.local_buffer_page.unwrap(),
        ))
    }
}
