use std::{ops::Range, ptr::NonNull};

use uwugc::ObjectPtr;

use crate::{
    descriptor,
    types::{TypeInfo, TypeInfoImpl},
};

enum PointerIteratorData<'a> {
    NormalIterator(descriptor::PointerIter<'a>),
    RefArray(Range<usize>),
}

pub struct PointerIterator<'a> {
    base: ObjectPtr,
    data: PointerIteratorData<'a>,
}

impl<'a> PointerIterator<'a> {
    pub fn from_type_info(info: &'a TypeInfo<'a>, ptr: ObjectPtr) -> Self {
        let data = match &info.0 {
            TypeInfoImpl::DynamicallyKnown(desc) => {
                PointerIteratorData::NormalIterator(desc.iter_ptrs())
            }
            TypeInfoImpl::StaticallyKnown(desc) => {
                PointerIteratorData::NormalIterator(desc.iter_ptrs())
            }
            TypeInfoImpl::RefArray(len) => PointerIteratorData::RefArray(0..*len),
        };

        Self { base: ptr, data }
    }
}

impl Iterator for PointerIterator<'_> {
    type Item = NonNull<u8>;

    fn next(&mut self) -> Option<Self::Item> {
        match &mut self.data {
            PointerIteratorData::NormalIterator(iter) => iter.next(),
            PointerIteratorData::RefArray(iter) => iter.next(),
        }
        .map(|x| {
            // SAFETY: We got field offset frm trusted sources like RefArray its essentially
            // every entry in array and for descriptor, descriptor maker already make sure its safe
            unsafe { self.base.data().byte_add(x) }
        })
    }
}
