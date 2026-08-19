use std::mem::MaybeUninit;

use uwugc_plus::{GCBox, HasDescriptor};

#[derive(HasDescriptor)]
pub struct Vec<T: Unpin + HasDescriptor + 'static> {
    // This is safe because all zeros pattern is valid
    // if there pointer, GC would ignore nulls. Because
    // we're not getting &T there no UB if all zeros is
    // invalid
    backing: GCBox<[MaybeUninit<T>]>,
    capacity: usize,
    len: usize,
}
