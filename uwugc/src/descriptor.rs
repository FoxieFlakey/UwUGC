use portable_atomic::AtomicPtr;

use crate::objects_manager::Object;

pub struct Field {
  pub offset: usize
}

pub struct Descriptor {
  pub fields: Vec<Field>
}

impl Descriptor {
  // SAFETY: Caller must properly match the descriptor,
  // to applicate descriptor for data pointed by raw pointer
  pub unsafe fn trace(&self, data: *const (), mut tracer: impl FnMut(&AtomicPtr<Object>)) {
    for field in &self.fields {
      let field_ptr = (data as usize + field.offset) as *const AtomicPtr<Object>;
      // SAFETY: The code which constructs this descriptor must give correct offsets
      //
      // in this program/library, it is ensured to be safe because the underlying type
      // must also implements Describeable which is unsafe trait because of that and
      // safety of this is responsibility of the implementor, and its impossible to pass
      // any constructed descriptor from safe code to create arbitrary potentially unsafe
      // descriptors for object allocations
      tracer(unsafe { &*field_ptr });
    }
  }
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

