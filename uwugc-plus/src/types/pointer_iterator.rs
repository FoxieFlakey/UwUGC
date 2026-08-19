use std::{ops::Range, ptr::NonNull};

use uwugc::ObjectPtr;

use crate::{
    Descriptor,
    array_metadata::ArrayHeader,
    descriptor,
    types::{TypeInfo, TypeInfoImpl},
};

enum PointerIteratorData<'a> {
    NormalIterator(descriptor::PointerIter<'a>),
    // This assume specific array layout where
    // the object is prepended with ArrayHeader
    // before following with data
    Array(ArrayIter<'a>),
    RefArray(Range<usize>),
}

struct ArrayIter<'a> {
    element_desc: &'a Descriptor,
    len: usize,
    current: usize,
    current_iter: descriptor::PointerIter<'a>,
}

pub struct PointerIterator<'a> {
    base: ObjectPtr,
    data: PointerIteratorData<'a>,
}

impl<'a> PointerIterator<'a> {
    pub fn from_type_info(info: &'a TypeInfo<'a>, ptr: ObjectPtr) -> Self {
        let data = match &info.0 {
            TypeInfoImpl::DynamicallyKnown(desc) => {
                if desc.is_array {
                    // SAFETY: Code which make Descriptor ensures that the array is prepended
                    // with ArrayHeader
                    let meta = unsafe { ptr.data().cast::<ArrayHeader>().as_ref() };
                    PointerIteratorData::Array(ArrayIter {
                        current: 0,
                        len: meta.size,
                        element_desc: desc,
                        current_iter: desc.iter_ptrs(),
                    })
                } else {
                    PointerIteratorData::NormalIterator(desc.iter_ptrs())
                }
            }
            TypeInfoImpl::StaticallyKnown(desc) => {
                if desc.is_array {
                    // SAFETY: Code which make Descriptor ensures that the array is prepended
                    // with ArrayHeader
                    let meta = unsafe { ptr.data().cast::<ArrayHeader>().as_ref() };
                    PointerIteratorData::Array(ArrayIter {
                        current: 0,
                        len: meta.size,
                        element_desc: desc,
                        current_iter: desc.iter_ptrs(),
                    })
                } else {
                    PointerIteratorData::NormalIterator(desc.iter_ptrs())
                }
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
            PointerIteratorData::Array(state) => loop {
                if state.current >= state.len {
                    break None;
                }

                let Some(offset) = state.current_iter.next() else {
                    state.current += 1;
                    state.current_iter = state.element_desc.iter_ptrs();
                    continue;
                };

                break Some(
                    (state.current * state.element_desc.size) + offset + size_of::<ArrayHeader>(),
                );
            },
        }
        .map(|x| {
            // SAFETY: We got field offset frm trusted sources like RefArray its essentially
            // every entry in array and for descriptor, descriptor maker already make sure its safe
            unsafe { self.base.data().byte_add(x) }
        })
    }
}
