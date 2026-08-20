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

impl<T> SafepointMut for T
where
    T: Safepoint,
{
    fn after_safepoint(&mut self) {
        (self as &dyn Safepoint).after_safepoint();
    }

    fn before_safepoint(&mut self) {
        (self as &dyn Safepoint).before_safepoint();
    }
}

impl<'a, T> Safepoint for T
where
    T: AsRef<[&'a RootRefRaw]>,
{
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
        [
            $(
                $crate::RootRef::as_raw($item)
            )*
        ]
    };
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
