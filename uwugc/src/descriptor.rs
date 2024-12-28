// NOTE: Everything in this file is considered to be part of public API

use std::{alloc::Layout, ops::{Deref, DerefMut}, ptr::{self, NonNull}};

use portable_atomic::AtomicPtr;

use crate::objects_manager::Object;

pub struct Field {
  pub offset: usize
}

pub struct DescriptorAPI {
  pub fields: Option<&'static [Field]>,
  pub layout: Layout,
  pub has_drop: bool
}

pub struct DescriptorInternal {
  pub api: DescriptorAPI,
  
  // A drop helper to help drop data region
  // it is placed here because all objects of
  // same descriptor are the same type
  //
  // the function can cast the "void pointer"
  // into what it needs
  pub drop_helper: fn(*mut ())
}

impl DerefMut for DescriptorInternal {
  fn deref_mut(&mut self) -> &mut Self::Target {
    &mut self.api
  }
}

impl Deref for DescriptorInternal {
  type Target = DescriptorAPI;
  
  fn deref(&self) -> &Self::Target {
    &self.api
  }
}

// The root descriptor which describes well itself
pub static SELF_DESCRIPTOR: DescriptorInternal = DescriptorInternal {
  api: DescriptorAPI {
    fields: None,
    layout: Layout::new::<DescriptorInternal>(),
    has_drop: false
  },
  drop_helper: DescriptorInternal::drop_helper
};

impl DescriptorInternal {
  // Caller must properly match the descriptor to correct type so trace can
  // correctly get pointer to fields
  pub(crate) unsafe fn trace(&self, data: NonNull<()>, mut tracer: impl FnMut(&AtomicPtr<Object>)) {
    let Some(fields) = self.fields else { return };
    for field in fields {
      // SAFETY: The code which constructs this descriptor must give correct offsets
      //
      // in this program/library, it is ensured to be safe because the underlying type
      // must also implements Describeable which is unsafe trait because of that and
      // safety of this is responsibility of the implementor, and its impossible to pass
      // any constructed descriptor from safe code to create arbitrary potentially unsafe
      // descriptors for object allocations
      tracer(unsafe { data.byte_add(field.offset).cast::<AtomicPtr<Object>>().as_ref() });
    }
  }
  
  fn drop_helper(this: *mut ()) {
    // SAFETY: Caller already make sure that 'this' is correct type
    // via SELF_DESCRIPTOR which describe itself
    unsafe { ptr::drop_in_place(this.cast::<Self>()) };
  }
}

/// The result will be shared by heaps
/// therefore has to live for 'static
/// because don't know how long those
/// heaps lives
///
/// # Safety
///
/// Unsafe because implementer has to
/// give correct Descriptor for a type
/// as incorrect descriptor cause unsafety
/// in GC during marking process as
/// Descriptor is only way GC knows how
/// to the read the data
pub unsafe trait Describeable {
  fn get_descriptor() -> DescriptorAPI;
}

// Few explicit blanket implementations
// because it can't be safety implemented
// for all types by genericly
macro_rules! impl_for_trait {
  ($trait_name:ident) => {
    unsafe impl<T: $trait_name> Describeable for T {
      #[inline]
      fn get_descriptor() -> DescriptorAPI {
        DescriptorAPI {
          fields: None,
          layout: Layout::new::<T>(),
          has_drop: false
        }
      }
    }
  };
}

// Copy-able type won't have descriptor,
// as future GCRef<T> (for reference in objects)
// will be !Copy and !Clone, therefore copy-able
// type never have GCRef<T>
impl_for_trait!(Copy);

