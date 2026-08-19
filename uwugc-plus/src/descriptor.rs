use std::{borrow::Cow, marker::PhantomData, slice};

use smallvec::{SmallVec, smallvec};

#[derive(Clone)]
pub struct Descriptor {
    // Field must contains offset to GCBox<T> fields inside an object
    pub fields: Cow<'static, [usize]>,

    // Due lack of Rust const power, I couldn't flatten this at compile
    // time. So it has to be flatten at runtime. This field supplements
    // the 'fields' fields
    pub unflattened: &'static [(usize, &'static Descriptor)],
    pub size: usize,

    // If this is true, that mean current Descriptor is [T]
    // where the rest of field is exact same values from T.
    // In "array descriptor" it is assume the data prepended
    // by ArrayHeader
    pub(crate) is_array: bool,
    _private: PhantomData<()>,
}

impl Descriptor {
    // # Safety
    // By creating this Descriptor, caller make sure that fields, size
    // and other details are consistent.
    //
    // Why here? there no unsafe code. I essentially pushed up the requirement
    // up to here. Because everything else depends Descriptor being sane
    pub const unsafe fn new(fields: Cow<'static, [usize]>, size: usize) -> Self {
        Self {
            fields,
            size,
            is_array: false,
            unflattened: &[],
            _private: PhantomData,
        }
    }

    pub const unsafe fn new_unflattened(
        fields: Cow<'static, [usize]>,
        size: usize,
        unflattened: &'static [(usize, &'static Descriptor)],
    ) -> Self {
        Self {
            fields,
            size,
            is_array: false,
            unflattened,
            _private: PhantomData,
        }
    }

    pub fn iter_ptrs<'a>(&'a self) -> PointerIter<'a> {
        PointerIter {
            iter_stack: smallvec![CurrentDescriptor {
                base: 0,
                fields: self.fields.iter(),
                childs: self.unflattened.iter(),
            }],
        }
    }

    pub(crate) const fn to_array(&'static self) -> Self {
        Self {
            fields: Cow::Borrowed(match &self.fields {
                Cow::Borrowed(x) => x,
                Cow::Owned(_) => panic!("Some of descriptor has owned fields"),
            }),
            is_array: true,
            size: self.size,
            unflattened: self.unflattened,
            _private: PhantomData,
        }
    }
}

struct CurrentDescriptor<'a> {
    base: usize,
    fields: slice::Iter<'a, usize>,
    childs: slice::Iter<'a, (usize, &'a Descriptor)>,
}

pub struct PointerIter<'a> {
    iter_stack: SmallVec<[CurrentDescriptor<'a>; 4]>,
}

impl Iterator for PointerIter<'_> {
    type Item = usize;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let current = self.iter_stack.last_mut()?;
            if let Some(field) = current.fields.next() {
                return Some(current.base + *field);
            }

            let Some((offset, child)) = current.childs.next() else {
                self.iter_stack.pop();
                continue;
            };

            self.iter_stack.push(CurrentDescriptor {
                base: *offset,
                childs: child.unflattened.iter(),
                fields: child.fields.iter(),
            });
        }
    }
}
