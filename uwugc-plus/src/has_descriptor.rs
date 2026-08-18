use crate::types::Descriptor;
use std::{
    borrow::Cow,
    cell::{Cell, Ref, RefCell, RefMut},
    collections::{HashMap, HashSet, VecDeque},
    ptr::NonNull,
    rc::Rc,
    sync::{
        Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard,
        atomic::{
            AtomicBool, AtomicI8, AtomicI16, AtomicI32, AtomicI64, AtomicIsize, AtomicU8,
            AtomicU16, AtomicU32, AtomicU64, AtomicUsize,
        },
    },
};

#[cfg(target_has_atomic = "128")]
use std::sync::atomic::{AtomicI128, AtomicU128};

// # Safety
// caller have to make sure the descriptor has
// valid information in accordance to Descriptor
// for Self. By implementing thing you are making
// sure that GCBox fileds inside object won't be
// assignment directly after construction. Must
// use .load() and .store() on it
//
// Few things is banend is on some bit pattenr GCBox
// is not present like Option<GCBox<..>> or any case
// where GCBox might not present is illegal. For nullable
// use GCBoxNullable instead
pub unsafe trait HasDescriptor: Unpin {
    const DESCRIPTOR: &'static Descriptor;
}

macro_rules! decl_static_array {
    ($primitive:ty) => {
        unsafe impl<const N: usize> HasDescriptor for [$primitive; N] {
            const DESCRIPTOR: &'static Descriptor =
                &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
        }
    };
}

macro_rules! decl_primitive {
    ($primitive:ty) => {
        unsafe impl HasDescriptor for $primitive {
            const DESCRIPTOR: &'static Descriptor =
                &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
        }

        decl_static_array!($primitive);
    };
}

decl_primitive!(bool);
decl_primitive!(u8);
decl_primitive!(u16);
decl_primitive!(u32);
decl_primitive!(u64);
decl_primitive!(u128);
decl_primitive!(usize);
decl_primitive!(i8);
decl_primitive!(i16);
decl_primitive!(i32);
decl_primitive!(i64);
decl_primitive!(i128);
decl_primitive!(isize);

decl_primitive!(AtomicBool);
decl_primitive!(AtomicU8);
decl_primitive!(AtomicU16);
decl_primitive!(AtomicU32);
decl_primitive!(AtomicU64);
#[cfg(target_has_atomic = "128")]
decl_primitive!(AtomicU128);
decl_primitive!(AtomicUsize);
decl_primitive!(AtomicI8);
decl_primitive!(AtomicI16);
decl_primitive!(AtomicI32);
decl_primitive!(AtomicI64);
#[cfg(target_has_atomic = "128")]
decl_primitive!(AtomicI128);
decl_primitive!(AtomicIsize);

// For some additional types like Result, Option, etc
// only if its generic match requirement

macro_rules! decl_generic {
    ($type:ident, $generic_one:ident with $lifetime:lifetime) => {
        unsafe impl<$lifetime, $generic_one> $crate::has_descriptor::HasDescriptor
            for $type<$lifetime, $generic_one>
        where
            $generic_one: Unpin + Copy + Clone + 'static,
        {
            const DESCRIPTOR: &'static $crate::types::Descriptor = &unsafe {
                $crate::types::Descriptor::new(::std::borrow::Cow::Borrowed(&[]), size_of::<Self>())
            };
        }
    };

    ($type:ident, $generic_one:ident) => {
        unsafe impl<$generic_one> $crate::has_descriptor::HasDescriptor for $type<$generic_one>
        where
            $generic_one: Unpin + Copy + Clone + 'static,
        {
            const DESCRIPTOR: &'static $crate::types::Descriptor = &unsafe {
                $crate::types::Descriptor::new(::std::borrow::Cow::Borrowed(&[]), size_of::<Self>())
            };
        }
    };

    ($type:ident, $generic_one:ident, $generic_two:ident) => {
        unsafe impl<$generic_one, $generic_two> $crate::has_descriptor::HasDescriptor
            for $type<$generic_one, $generic_two>
        where
            $generic_one: Unpin + Copy + Clone + 'static,
            $generic_two: Unpin + Copy + Clone + 'static,
        {
            const DESCRIPTOR: &'static $crate::types::Descriptor = &unsafe {
                $crate::types::Descriptor::new(::std::borrow::Cow::Borrowed(&[]), size_of::<Self>())
            };
        }
    };
}

decl_generic!(Option, T);
decl_generic!(Rc, T);
decl_generic!(Arc, T);
decl_generic!(RefCell, T);
decl_generic!(Mutex, T);
decl_generic!(Box, T);
decl_generic!(Cell, T);
decl_generic!(RwLock, T);
decl_generic!(Ref, T with 'a);
decl_generic!(RefMut, T with 'a);
decl_generic!(MutexGuard, T with 'a);
decl_generic!(RwLockReadGuard, T with 'a);
decl_generic!(RwLockWriteGuard, T with 'a);
decl_generic!(Cow, T with 'a);
decl_generic!(Vec, T);
decl_generic!(VecDeque, T);
decl_generic!(NonNull, T);
decl_generic!(HashSet, T);
decl_generic!(HashMap, K, V);

mod parking {
    use parking_lot::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

    decl_generic!(Mutex, T);
    decl_generic!(RwLock, T);
    decl_generic!(MutexGuard, T with 'a);
    decl_generic!(RwLockReadGuard, T with 'a);
    decl_generic!(RwLockWriteGuard, T with 'a);
}

// Tuples
unsafe impl<T1, T2, T3, T4, T5, T6, T7, T8> HasDescriptor for (T1, T2, T3, T4, T5, T6, T7, T8)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
    T4: Unpin + Copy + Clone + 'static,
    T5: Unpin + Copy + Clone + 'static,
    T6: Unpin + Copy + Clone + 'static,
    T7: Unpin + Copy + Clone + 'static,
    T8: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2, T3, T4, T5, T6, T7> HasDescriptor for (T1, T2, T3, T4, T5, T6, T7)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
    T4: Unpin + Copy + Clone + 'static,
    T5: Unpin + Copy + Clone + 'static,
    T6: Unpin + Copy + Clone + 'static,
    T7: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2, T3, T4, T5, T6> HasDescriptor for (T1, T2, T3, T4, T5, T6)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
    T4: Unpin + Copy + Clone + 'static,
    T5: Unpin + Copy + Clone + 'static,
    T6: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2, T3, T4, T5> HasDescriptor for (T1, T2, T3, T4, T5)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
    T4: Unpin + Copy + Clone + 'static,
    T5: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2, T3, T4> HasDescriptor for (T1, T2, T3, T4)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
    T4: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2, T3> HasDescriptor for (T1, T2, T3)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
    T3: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl<T1, T2> HasDescriptor for (T1, T2)
where
    T1: Unpin + Copy + Clone + 'static,
    T2: Unpin + Copy + Clone + 'static,
{
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}

unsafe impl HasDescriptor for () {
    const DESCRIPTOR: &'static Descriptor =
        &unsafe { Descriptor::new(Cow::Borrowed(&[]), size_of::<Self>()) };
}
