pub struct Field {
  pub offset: usize
}

pub struct Descriptor {
  pub fields: Vec<Field>
}

// The result will be shared by heaps
// therefore has to live for 'static
// because don't know how long those
// heaps lives
//
// Unsafe because implementer has to
// give correct Descriptor for a type
// as incorrect descriptor cause unsafety
// in GC during marking process as
// Descriptor is only way GC knows how
// to the read the data
pub unsafe trait Describeable {
  fn get_descriptor() -> Option<&'static Descriptor>;
}

// Few explicit blanket implementations
// because it can't be safety implemented
// for all types by genericly
macro_rules! impl_for_trait {
  ($trait_name:ident) => {
    unsafe impl<T: $trait_name> Describeable for T {
      fn get_descriptor() -> Option<&'static Descriptor> {
        return None;
      }
    }
  };
}

// Copy-able type won't have descriptor,
// as future GCRef<T> (for reference in objects)
// will be !Copy and !Clone, therefore copy-able
// type never have GCRef<T>
impl_for_trait!(Copy);

