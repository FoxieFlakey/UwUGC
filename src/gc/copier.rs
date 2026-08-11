use crate::gc::{HeapInfoLater, relocation_map::FrozenRegistry};

pub struct Copier {}

impl Copier {
    pub fn new() -> Self {
        Self {}
    }

    // note, heap_len may be larger than last byte in compacted form
    pub fn start(self, heap: HeapInfoLater, reloc_registry: FrozenRegistry) -> CopierActive {
        // TODO: Do something
        CopierActive {
            state: self,
            reloc_registry,
            heap,
        }
    }
}

pub struct CopierActive {
    state: Copier,
    reloc_registry: FrozenRegistry,
    #[expect(unused)]
    heap: HeapInfoLater,
}

impl CopierActive {
    pub fn finish(self) -> (FrozenRegistry, Copier) {
        (self.reloc_registry, self.state)
    }
}
