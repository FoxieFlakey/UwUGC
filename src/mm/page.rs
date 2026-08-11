use std::ptr::NonNull;

// FlexPage is page that can be made out of one or more base pages (which
// will be called just page)
#[derive(Clone)]
pub struct FlexPage {
    kind: FlexPageKind,
    pub(super) start: NonNull<u8>,
    used_bytes: usize,
}

impl FlexPage {
    pub(super) fn new(kind: FlexPageKind, start: *mut u8) -> Self {
        Self {
            kind,
            start: NonNull::new(start).unwrap(),
            used_bytes: 0,
        }
    }

    pub fn start(&self) -> *mut u8 {
        self.start.as_ptr()
    }

    pub fn size(&self) -> usize {
        self.kind.nr_pages() * BASE_PAGE_SIZE
    }

    pub fn nr_pages(&self) -> usize {
        self.kind.nr_pages()
    }

    pub fn used(&self) -> usize {
        self.used_bytes
    }

    pub fn free(&self) -> usize {
        self.size() - self.used()
    }

    #[expect(unused)]
    pub fn kind(&self) -> FlexPageKind {
        self.kind
    }

    pub fn alloc(&mut self, size: usize) -> Option<*mut u8> {
        if self.free() >= size {
            let ret = self.start.as_ptr().wrapping_byte_add(self.used_bytes);
            self.used_bytes += size;
            Some(ret)
        } else {
            None
        }
    }
}

pub const BASE_PAGE_SIZE: usize = 2 * 1024 * 1024;
pub const BASE_PAGE_SHIFT: u32 = 21;

#[derive(Clone, Copy)]
pub enum FlexPageKind {
    Small,
    Medium,
    Huge { base_count: usize },
}

unsafe impl Send for FlexPage {}
unsafe impl Sync for FlexPage {}

impl FlexPageKind {
    pub fn nr_pages(&self) -> usize {
        match self {
            FlexPageKind::Small => 1,
            FlexPageKind::Medium => 4,
            FlexPageKind::Huge { base_count } => *base_count,
        }
    }

    pub fn max_object_size(&self) -> usize {
        match self {
            FlexPageKind::Small => 256 * 1024,
            FlexPageKind::Medium => 1024 * 1024,
            FlexPageKind::Huge { .. } => usize::MAX,
        }
    }
}
