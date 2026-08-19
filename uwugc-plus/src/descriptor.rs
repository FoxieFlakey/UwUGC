use std::{borrow::Cow, marker::PhantomData};

#[derive(Clone)]
pub struct Descriptor {
    // Field must contains offset to GCBox<T> fields inside an object
    pub fields: Cow<'static, [usize]>,

    // Due lack of Rust const power, I couldn't flatten this at compile
    // time. So it has to be flatten at runtime. This field supplements
    // the 'fields' fields
    pub unflattened: &'static [ (usize, &'static Descriptor) ],
    pub size: usize,
    _private: PhantomData<()>,
}

impl Descriptor {
    // # Safety
    // By creating this Descriptor, caller make sure that fields, size
    // and other details are consistent.
    //
    // Why here? there no unsafe code. I essentially pushed up the requirement
    // up to here. Because everything else depends Descriptor being sane
    pub const unsafe fn new(
        fields: Cow<'static, [usize]>,
        size: usize,
    ) -> Self {
        Self {
            fields,
            size,
            unflattened: &[],
            _private: PhantomData,
        }
    }

    pub const unsafe fn new_unflattened(
        fields: Cow<'static, [usize]>,
        size: usize,
        unflattened: &'static [ (usize, &'static Descriptor) ]
    ) -> Self {
        Self {
            fields,
            size,
            unflattened,
            _private: PhantomData,
        }
    }
}

