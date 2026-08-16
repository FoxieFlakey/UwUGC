// Copy from older version. So i dont need to rewrite relocation map again.
// https://github.com/FoxieFlakey/UwUGC/blob/f2cfca281a9ba929f040aa1092a482fe1d661081/uwugc/src/gc/mover/reloc_record.rs
//
// Its also adapted so .insert(..) is less strict about ordering

use std::{cmp::Ordering, ops::Range};

// Registry mainly handles stuffs like
// 1. Getting record which is intersecting with
//    a given destination range
// 2. Given any pointer (in source space) which could points to middle of
//    any record. Return moved pointer after record is moved and properly
//    offseted

// Note: multiple objects that is moved together can be coalesced to be one
// record, the mover does not need 1 to 1 mapping to the relocation record
#[derive(Clone)]
pub struct RelocationRecord {
    pub src: usize,
    pub dest: usize,
    pub size: usize,
}

impl RelocationRecord {
    pub fn get_dest_range(&self) -> Range<usize> {
        Range {
            start: self.dest,
            end: (self.dest + self.size),
        }
    }

    pub fn get_src_range(&self) -> Range<usize> {
        Range {
            start: self.src,
            end: (self.src + self.size),
        }
    }

    pub fn map_src_to_dest(&self, ptr: usize) -> usize {
        assert!(
            self.get_src_range().contains(&ptr),
            "Pointer to be mapped is not in source"
        );
        self.dest + (ptr - self.src)
    }
}

pub struct RegistryBuilder {
    data: FrozenRegistry,
}

pub struct FrozenRegistry {
    records_sorted_by_src: Vec<RelocationRecord>,
    records_sorted_by_dest: Vec<RelocationRecord>,

    starting_src: Option<usize>,
    starting_dest: Option<usize>,
    latest_free_src: Option<usize>,
    latest_free_dest: Option<usize>,
}

impl RegistryBuilder {
    pub fn new() -> Self {
        Self {
            data: FrozenRegistry {
                latest_free_dest: None,
                latest_free_src: None,
                starting_dest: None,
                starting_src: None,
                records_sorted_by_dest: Vec::new(),
                records_sorted_by_src: Vec::new(),
            },
        }
    }

    pub fn insert(&mut self, record: RelocationRecord) {
        if let Some(x) = self.data.latest_free_dest {
            if record.dest >= x {
                self.data.latest_free_dest = Some(record.get_dest_range().end);
            }
        } else {
            self.data.latest_free_dest = Some(record.get_dest_range().end);
        }

        if let Some(x) = self.data.latest_free_src {
            if record.src >= x {
                self.data.latest_free_src = Some(record.get_src_range().end);
            }
        } else {
            self.data.latest_free_src = Some(record.get_src_range().end);
        }

        if self.data.starting_src.is_none() {
            self.data.starting_src = Some(record.get_src_range().start);
        }

        if self.data.starting_dest.is_none() {
            self.data.starting_dest = Some(record.get_dest_range().start);
        }

        self.data.records_sorted_by_src.push(record.clone());
        self.data.records_sorted_by_dest.push(record);
    }

    pub fn freeze(mut self) -> FrozenRegistry {
        // Source records might not sorted yet
        self.data.records_sorted_by_src.sort_by_key(|x| x.src);
        self.data
    }
}

impl FrozenRegistry {
    pub fn find_record_for_containing_src(&self, src_ptr: usize) -> Option<&RelocationRecord> {
        self.iterate_records_in_src_range(&Range {
            start: src_ptr,
            end: src_ptr + 1,
        })
        .next()
    }

    pub fn map_src_to_dest(&self, ptr: usize) -> Option<usize> {
        self.find_record_for_containing_src(ptr)
            .map(|record| record.map_src_to_dest(ptr))
    }

    fn iterate_records_in_range_by<'a, F>(
        vec: &'a Vec<RelocationRecord>,
        mut extractor: F,
        range: &Range<usize>,
    ) -> impl Iterator<Item = &'a RelocationRecord>
    where
        F: FnMut(&RelocationRecord) -> Range<usize>,
    {
        vec.binary_search_by(|probe| {
            let probe_range = extractor(probe);
            if probe_range.contains(&range.start) {
                Ordering::Equal
            } else if probe_range.end <= range.start {
                Ordering::Less
            } else if probe_range.start > range.start {
                Ordering::Greater
            } else {
                unimplemented!("unknown corner case");
            }
        })
        .ok()
        .into_iter()
        .map(|start_idx| vec[start_idx..].iter())
        .flatten()
        .take_while(move |record| extractor(record).start < range.end)
    }

    pub fn iterate_records_in_dest_range(
        &self,
        dest: &Range<usize>,
    ) -> impl Iterator<Item = &RelocationRecord> {
        Self::iterate_records_in_range_by(
            &self.records_sorted_by_dest,
            |record| record.get_dest_range(),
            dest,
        )
    }

    pub fn iterate_records_in_src_range(
        &self,
        src: &Range<usize>,
    ) -> impl Iterator<Item = &RelocationRecord> {
        Self::iterate_records_in_range_by(
            &self.records_sorted_by_src,
            |record| record.get_src_range(),
            src,
        )
    }

    pub fn unfreeze(mut self) -> RegistryBuilder {
        self.latest_free_dest = None;
        self.latest_free_src = None;
        self.starting_src = None;
        self.starting_dest = None;
        self.records_sorted_by_dest.clear();
        self.records_sorted_by_src.clear();

        RegistryBuilder { data: self }
    }
}
