use crate::{RootRef, RootRefRaw};

// Safe point for types that don't need &mut self
pub trait Safepoint {
    fn before_safepoint(&self);
    fn after_safepoint(&self);
}

pub trait SafepointMut {
    fn before_safepoint(&mut self);
    fn after_safepoint(&mut self);
}

impl Safepoint for &[&'_ RootRefRaw] {
    fn before_safepoint(&self) {
        self.as_ref().iter().for_each(|x| {
            x.store();
        });
    }

    fn after_safepoint(&self) {
        self.as_ref().iter().for_each(|x| {
            x.load();
        });
    }
}

// Root ref can directly be used for safepoint for saving only one root ref
impl<T: Unpin + ?Sized> Safepoint for RootRef<T> {
    fn before_safepoint(&self) {
        RootRef::as_raw(self).store();
    }

    fn after_safepoint(&self) {
        RootRef::as_raw(self).load();
    }
}

// Convenient macro for saving/loading root refs
#[macro_export]
macro_rules! safe_roots {
    ($($item:expr),* $(,)?) => {
        &$crate::RootList([
            $(
                $crate::RootRef::as_raw($item)
            )*
        ])
    };
}

impl<T: ?Sized + Safepoint> Safepoint for &T {
    fn before_safepoint(&self) { (**self).before_safepoint(); }
    fn after_safepoint(&self) { (**self).after_safepoint(); }
}

impl<T: ?Sized + Safepoint> Safepoint for &mut T {
    fn before_safepoint(&self) { (**self).before_safepoint(); }
    fn after_safepoint(&self) { (**self).after_safepoint(); }
}

impl<T: ?Sized + Safepoint> SafepointMut for &T {
    fn before_safepoint(&mut self) { Safepoint::before_safepoint(*self); }
    fn after_safepoint(&mut self) { Safepoint::after_safepoint(*self); }
}

impl<T: ?Sized + SafepointMut> SafepointMut for &mut T {
    fn before_safepoint(&mut self) { (**self).before_safepoint(); }
    fn after_safepoint(&mut self) { (**self).after_safepoint(); }
}

pub struct RootList<'a, const N: usize>(pub [&'a RootRefRaw; N]);

impl<const N: usize> Safepoint for RootList<'_, N> {
    fn before_safepoint(&self) {
        self.0.iter().for_each(|x| x.store());
    }

    fn after_safepoint(&self) {
        self.0.iter().for_each(|x| x.load());
    }
}

pub struct SafepointList<'a>(pub &'a mut [&'a mut dyn SafepointMut]);

impl SafepointMut for SafepointList<'_> {
    fn before_safepoint(&mut self) {
        self.0.iter_mut().for_each(|x| {
            x.before_safepoint();
        });
    }

    fn after_safepoint(&mut self) {
        self.0.iter_mut().for_each(|x| {
            x.after_safepoint();
        });
    }
}
