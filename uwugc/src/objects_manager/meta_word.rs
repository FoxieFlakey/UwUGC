use std::{marker::PhantomData, ptr::{self, NonNull}, sync::atomic::Ordering};

use portable_atomic::AtomicPtr;

use crate::descriptor::{self, DescriptorInternal};

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

#[non_exhaustive]
pub enum ObjectMetadata<'word> {
  // Corresponds to ORDINARY_OBJECT_BIT set
  Ordinary(OrdinaryObjectMetadata<'word>)
}

#[derive(Debug)]
pub enum GetDescriptorError {
  IncorrectObjectType
}

pub struct OrdinaryObjectMetadata<'word> {
  descriptor: Option<NonNull<Object>>,
  _phantom: PhantomData<&'word ()>
}

impl<'word> OrdinaryObjectMetadata<'word> {
  pub fn get_descriptor(&self) -> &'word DescriptorInternal {
    if let Some(obj_ptr) = self.descriptor {
      // SAFETY: The metaword constructor's caller already guarantee that the descriptor
      // pointer valid as long MetaWord exists and correct type of object too
      unsafe { Object::get_raw_ptr_to_data(obj_ptr).cast::<DescriptorInternal>().as_ref() }
    } else {
      &descriptor::SELF_DESCRIPTOR
    }
  }
  
  pub fn is_descriptor(&self) -> bool {
    self.descriptor.is_none()
  }
  
  pub fn get_descriptor_obj(&self) -> Option<NonNull<Object>> {
    self.descriptor
  }
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
  
  pub fn get_object_metadata(&self) -> ObjectMetadata {
    let word = self.word.load(Ordering::Relaxed);
    if word.addr() & ORDINARY_OBJECT_BIT != 0  {
      return ObjectMetadata::Ordinary(OrdinaryObjectMetadata {
        descriptor: NonNull::new(self.word.load(Ordering::Relaxed).map_addr(|x| x & DATA_MASK)),
        _phantom: PhantomData
      });
    }
    
    unimplemented!();
  }
  
  pub fn is_descriptor(&self) -> Result<bool, GetDescriptorError> {
    match self.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => Ok(meta.is_descriptor())
    }
  }
  
  pub fn get_descriptor_obj_ptr(&self) -> Result<Option<NonNull<Object>>, GetDescriptorError> {
    match self.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => Ok(meta.get_descriptor_obj()),
      
      // Currently there no other type implemented yet
      // so catch all arm here to silence clippy warning
      #[expect(unreachable_patterns)]
      _ => Err(GetDescriptorError::IncorrectObjectType)
    }
  }
  
  pub fn get_descriptor(&self) -> Result<&DescriptorInternal, GetDescriptorError> {
    match self.get_object_metadata() {
      ObjectMetadata::Ordinary(meta) => Ok(meta.get_descriptor())
    }
  }
}


