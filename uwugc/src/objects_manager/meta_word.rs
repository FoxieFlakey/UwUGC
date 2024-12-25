use std::ptr::{self, NonNull};

use crate::{descriptor, Descriptor};

use super::Object;

// It is a single "multi-purpose" descriptor pointer
pub struct MetaWord {
  word: *const Object
}

impl MetaWord {
  // SAFETY: Caller must ensure that 'desc' is pointer
  // to object with correct type or None if the meta word
  // is for object which is the descriptor itself
  //
  // Caller also have to make sure the 'desc' pointer given
  // is valid as long as the MetaWord exists
  pub unsafe fn new(desc: Option<NonNull<Object>>) -> MetaWord {
    MetaWord {
      word: desc
        .map(|ptr| ptr.as_ptr().cast_const())
        .unwrap_or(ptr::null())
    }
  }
  
  pub fn is_descriptor(&self) -> bool {
    self.word.is_null()
  }
  
  pub fn get_descriptor_obj_ptr(&self) -> Option<NonNull<Object>> {
    if self.is_descriptor() {
      return None;
    }
    
    NonNull::new(self.word.cast_mut())
  }
  
  pub fn get_descriptor(&self) -> &Descriptor {
    if let Some(obj_ptr) = self.get_descriptor_obj_ptr() {
      // SAFETY: The constructor's caller already guarantee that the descriptor
      // pointer valid as long MetaWord exists and correct type of object too
      unsafe { obj_ptr.as_ref().get_raw_ptr_to_data().cast::<Descriptor>().as_ref().unwrap_unchecked() } 
    } else {
      &descriptor::SELF_DESCRIPTOR
    }
  }
}


