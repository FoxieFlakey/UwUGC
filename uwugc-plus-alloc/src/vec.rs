use std::{
    ops::{Deref, DerefMut},
    slice,
};

use uwugc_plus::{
    Array, GCBoxOption, HasDescriptor, RootRef, SafepointList, SafepointMut, safe_roots,
};

use crate::zero_or_init::ZeroOrInit;

#[derive(HasDescriptor)]
pub struct Vec<T: Unpin + HasDescriptor + 'static> {
    // This is safe because all zeros pattern is valid
    // if there pointer, GC would ignore nulls. Because
    // we're not getting &T there no UB if all zeros is
    // invalid
    backing: GCBoxOption<Array<ZeroOrInit<T>>>,
    len: usize,
}

impl<T: Unpin + HasDescriptor + 'static> Vec<T> {
    pub fn new(safepoint: &mut dyn SafepointMut) -> Option<RootRef<Self>> {
        uwugc_plus::alloc(safepoint, 0, || Vec {
            backing: GCBoxOption::none(),
            len: 0,
        })
    }

    fn ensure_capacity(
        &mut self,
        safepoint: &mut dyn SafepointMut,
        min_capacity: usize,
    ) -> Result<(), ()> {
        let min_capacity = if min_capacity.is_power_of_two() {
            min_capacity
        } else {
            min_capacity.next_power_of_two()
        };

        if self.backing.get_mut().is_none() {
            let allocated =
                uwugc_plus::alloc_array(safepoint, 0, || ZeroOrInit::zeroed(), min_capacity)
                    .ok_or(())?;
            self.backing.store(Some(allocated));
        }

        let backing = self.backing.get_mut().unwrap();
        if min_capacity > backing.len() {
            let mut allocated =
                uwugc_plus::alloc_array(safepoint, 0, || ZeroOrInit::zeroed(), min_capacity)
                    .ok_or(())?;
            let old = self.backing.get_mut().unwrap();
            for (i, src) in old.iter_mut().enumerate() {
                allocated[i] = src.take();

                // Occasionally safepoint during potentially long copies
                if i % 512 != 0 {
                    uwugc_plus::safepoint(&mut SafepointList(&mut [
                        &mut safe_roots!(&mut allocated),
                        safepoint,
                    ]));
                }
            }

            self.backing.store(Some(allocated));
        }

        Ok(())
    }

    pub fn capacity(&self) -> usize {
        self.backing.get_ref().unwrap().len()
    }

    pub fn insert(&mut self, safepoint: &mut dyn SafepointMut, data: T) -> Result<(), ()> {
        self.ensure_capacity(safepoint, self.len + 1)?;
        self.backing.get_mut().unwrap()[self.len] = ZeroOrInit::new(data);
        self.len += 1;
        Ok(())
    }
}

impl<T: Unpin + HasDescriptor + 'static> Deref for Vec<T> {
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        let len = self.len;
        if len == 0 {
            return &[];
        }

        let ptr = self.backing.get_ref().unwrap().as_ptr().cast::<T>();

        // SAFETY: We have initialized the portion the slice representing
        unsafe { slice::from_raw_parts(ptr, len) }
    }
}

impl<T: Unpin + HasDescriptor + 'static> DerefMut for Vec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        let len = self.len;
        if len == 0 {
            return &mut [];
        }

        let ptr = self.backing.get_mut().unwrap().as_mut_ptr().cast::<T>();

        // SAFETY: We have initialized the portion the slice representing
        unsafe { slice::from_raw_parts_mut(ptr, len) }
    }
}
