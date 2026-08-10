use crate::gc::relocation_map::FrozenRegistry;

pub struct Copier {
}

impl Copier {
    pub fn new() -> Self {
        Self {}
    }

    // note, heap_len may be larger than last byte in compacted form
    pub fn start(self, heap_ptr: *mut u8, heap_len: usize, reloc_registry: FrozenRegistry) -> CopierActive {
        // TODO: Do something
        CopierActive {
            state: self,
            reloc_registry,
            heap_ptr,
            heap_len,
        }
    }
}

pub struct CopierActive {
    state: Copier,
    reloc_registry: FrozenRegistry,
    #[expect(unused)]
    heap_ptr: *mut u8,
    #[expect(unused)]
    heap_len: usize,
}

impl CopierActive {
    pub fn finish(self) -> (FrozenRegistry, Copier) {
        (self.reloc_registry, self.state)
    }
}



