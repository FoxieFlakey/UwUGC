use std::slice;

use crate::{
    gc::{HeapInfoLater, relocation_map::FrozenRegistry},
    mmap::Mmap,
};

pub struct Copier {}

impl Copier {
    pub fn new() -> Self {
        Self {}
    }

    // note, heap_len may be larger than last byte in compacted form
    // # Safety
    // Caller has to ensure the heap described by 'heap' is not
    // currently used for duration of this function
    pub unsafe fn start(
        self,
        heap: HeapInfoLater,
        reloc_registry: FrozenRegistry,
        from_mapping: Mmap,
    ) -> CopierActive {
        // Do dumb copying, leaving "later used" unmoved
        // later update to use userfaultfd

        // SAFETY: We own the mapping
        let src_slice =
            unsafe { slice::from_raw_parts(from_mapping.get_ptr(), from_mapping.len()) };
        // SAFETY: Caller make sure the destination heap is untouched
        let dest_slice = unsafe {
            slice::from_raw_parts_mut(heap.start, heap.compacted_end.addr() - heap.start.addr())
        };

        for record in reloc_registry
            .iterate_records_in_dest_range(&reloc_registry.get_dest_range().unwrap_or(0..0))
        {
            let src = &src_slice[record.src..record.src + record.size];
            let dest = &mut dest_slice[record.dest..record.dest + record.size];
            dest.copy_from_slice(src);
        }

        CopierActive {
            state: self,
            reloc_registry,
            heap,
            from_mapping,
        }
    }
}

pub struct CopierActive {
    state: Copier,
    reloc_registry: FrozenRegistry,
    #[expect(unused)]
    heap: HeapInfoLater,
    from_mapping: Mmap,
}

impl CopierActive {
    pub fn finish(self) -> (FrozenRegistry, Copier, Mmap) {
        (self.reloc_registry, self.state, self.from_mapping)
    }
}
