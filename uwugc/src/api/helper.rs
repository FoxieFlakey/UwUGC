
// Helper to export a type wrapped in structure where
// the field of inner type is pub(crate)
macro_rules! export_type_as_wrapper {
  ($($lifetime:lifetime),*, $name:ident, $internal_type:ty) => {
    pub struct $name<$($lifetime),*> {
      pub(crate) inner: $internal_type
    }
    
    impl<$($lifetime),*> $name<$($lifetime),*> {
      pub(crate) fn from(val: $internal_type) -> Self {
        return Self {
          inner: val
        }
      }
    }
  };
  
  ($name:ident, $internal_type:ty) => {
    pub struct $name {
      pub(crate) inner: $internal_type
    }
    
    impl $name {
      pub(crate) fn from(val: $internal_type) -> Self {
        return Self {
          inner: val
        }
      }
    }
  };
}

pub(crate) use export_type_as_wrapper;


