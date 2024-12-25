use std::{ptr::{self, NonNull}, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::{descriptor, Descriptor};

use super::Object;

// It is a single "multi-purpose" descriptor pointer
pub struct MetaWord {
  word: AtomicPtr<Object>
}

// Minimum alignment object needed to ensure
// bottom two bits remains unused and can be
// used for metadata purposes
const OBJECT_ALIGNMENT_SHIFT: u32 = 2;
const _: () = assert!(align_of::<Object>() >= (1 << OBJECT_ALIGNMENT_SHIFT), "Object is not in correct alignment! Correct alignment is REQUIRED for MetaWord to be correct");

// Lower two bit of properly aligned
// object pointer is used for metadata
const METADATA_MASK: usize = 0b11;
const DATA_MASK: usize = !METADATA_MASK;

const ORDINARY_OBJECT_BIT: usize = 0b01;
const MARK_BIT: usize            = 0b10;

enum ObjectType {
  // Corresponds to ORDINARY_OBJECT_BIT set
  Ordinary
}

impl MetaWord {
  // SAFETY: Caller must ensure that 'desc' is pointer
  // to object with correct type or None if the meta word
  // is for object which is the descriptor itself
  //
  // Caller also have to make sure the 'desc' pointer given
  // is valid as long as the MetaWord exists
  pub unsafe fn new(desc: Option<NonNull<Object>>, mark_bit: bool) -> MetaWord {
    MetaWord {
      word: AtomicPtr::new(
        desc
          .map_or(ptr::null_mut(), |ptr| {
            assert!(ptr.addr().trailing_zeros() >= OBJECT_ALIGNMENT_SHIFT, "Incorrect alignment was given!");
            ptr.as_ptr()
          })
          .map_addr(|mut x| {
            x |= ORDINARY_OBJECT_BIT;
            
            // Set the mark bit
            if mark_bit {
              x |= MARK_BIT;
            }
            
            x
          })
      )
    }
  }
  
  // Swap MARK_BIT part of metadata and return old one
  //
  // Mark bit is only part which changes throughout lifetime
  // of MetaWord, the rest don't change so CAS loop is unnecessary
  pub fn swap_mark_bit(&self, new_bit: bool) -> bool {
    let new = self.word.load(Ordering::Relaxed);
    let old = self.word.swap(new.map_addr(|mut x| {
      if new_bit {
        x |= MARK_BIT;
      } else {
        x &= !MARK_BIT;
      }
      
      x
    }), Ordering::Relaxed);
    
    (old.addr() & MARK_BIT) == MARK_BIT
  }
  
  pub fn get_mark_bit(&self) -> bool {
    (self.word.load(Ordering::Relaxed).addr() & MARK_BIT) == MARK_BIT
  }
  
  fn get_object_type(&self) -> ObjectType {
    let word = self.word.load(Ordering::Relaxed);
    if word.addr() & ORDINARY_OBJECT_BIT !=0  {
      return ObjectType::Ordinary;
    }
    
    unimplemented!();
  }
  
  pub fn is_descriptor(&self) -> bool {
    match self.get_object_type() {
      ObjectType::Ordinary => self.get_descriptor_obj_ptr().is_none()
    }
  }
  
  pub fn get_descriptor_obj_ptr(&self) -> Option<NonNull<Object>> {
    match self.get_object_type() {
      ObjectType::Ordinary => NonNull::new(self.word.load(Ordering::Relaxed).map_addr(|x| x & DATA_MASK))
    }
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


