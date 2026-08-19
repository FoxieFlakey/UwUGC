use std::ops::Range;

use either::Either;

use crate::{
    descriptor,
    types::{TypeInfo, TypeInfoImpl},
};

pub struct PointerIterator<'a>(Either<descriptor::PointerIter<'a>, Range<usize>>);

impl<'a> PointerIterator<'a> {
    pub fn from_type_info(info: &'a TypeInfo<'a>) -> Self {
        PointerIterator(match &info.0 {
            TypeInfoImpl::DynamicallyKnown(desc) => Either::Left(desc.iter_ptrs()),
            TypeInfoImpl::StaticallyKnown(desc) => Either::Left(desc.iter_ptrs()),
            TypeInfoImpl::RefArray(len) => Either::Right(0..*len),
        })
    }
}

impl Iterator for PointerIterator<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        self.0.next()
    }
}
