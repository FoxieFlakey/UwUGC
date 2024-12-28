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
const METADATA_MASK: usize       = 0b0000_0011;
const ORDINARY_OBJECT_BIT: usize = 0b0000_0001;
const MARK_BIT: usize            = 0b0000_0010;

const DATA_MASK: usize = !METADATA_MASK;

// When meta word represents non ordinary (ordinary is
// object which has descriptor present, so non ordinary
// has no descriptor, therefore opens data bits for other
// purpose)
//
// These masks must be used instead of above if ORDINARY_OBJECT_BIT
// is unset as the format is different than ordinary object
const NON_ORDINARY_METADATA_MASK: usize = 0b0000_0111;
const NON_ORDINARY_REF_ARRAY_BIT: usize = 0b0000_0100;

const NON_ORDINARY_DATA_MASK: usize = !NON_ORDINARY_METADATA_MASK;
const NON_ORDINARY_DATA_SHIFT: usize = 3;

// Largest value can be represented by data part of 'non ordinary' format
const NON_ORDINARY_DATA_MAX: usize = usize::MAX >> NON_ORDINARY_DATA_SHIFT;

#[derive(Debug)]
pub struct SizeTooBig;

pub enum ObjectMetadata<'word> {
  // Corresponds to ORDINARY_OBJECT_BIT set
  Ordinary(OrdinaryObjectMetadata<'word>),
  
  // Corresponds to ORDINARY_OBJECT_BIT unset and NON_ORDINARY_REF_ARRAY_BIT set
  ReferenceArray(ReferenceArrayMetadata)
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

pub struct ReferenceArrayMetadata {
  array_length: usize
}

impl ReferenceArrayMetadata {
  pub fn get_array_len(&self) -> usize {
    self.array_length
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
  
  pub fn new_array(array_len: usize, mark_bit: bool) -> Result<Self, SizeTooBig> {
    if array_len > NON_ORDINARY_DATA_MAX {
      return Err(SizeTooBig);
    }
    
    Ok(MetaWord {
      word: AtomicPtr::new(
        ptr::null_mut::<Object>()
          .map_addr(|mut x| {
            // Set the mark bit
            if mark_bit {
              x |= MARK_BIT;
            }
            
            // This is not ordinary object
            x &= !ORDINARY_OBJECT_BIT;
            x |= NON_ORDINARY_REF_ARRAY_BIT;
            x |= array_len << NON_ORDINARY_DATA_SHIFT;
            
            x
          })
      )
    })
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
      // Its an ordinary object with descriptor
      return ObjectMetadata::Ordinary(OrdinaryObjectMetadata {
        descriptor: NonNull::new(self.word.load(Ordering::Relaxed).map_addr(|x| x & DATA_MASK)),
        _phantom: PhantomData
      });
    } else {
      let word = word.addr();
      
      if word & NON_ORDINARY_REF_ARRAY_BIT != 0 {
        // Its a reference array
        return ObjectMetadata::ReferenceArray(ReferenceArrayMetadata {
          // This is able to represent all possible array length because each
          // pointer on 64-bit systems is 8 bytes which means arrays are always
          // multiple of 8 bytes in size which leaves with bottom 3 bits unused
          // and can be used for metadata and can be shifted to right effectively
          // (SIZE_IN_BYTES / 8) with SIZE_IN_BYTES always multiples of 8 and still
          // able to represent all possible array sizes (2^61 entries and 2^64
          // bytes of array)
          //
          // For 32-bit systems, sacrifice HAS TO BE MADE to maximum length of array
          // as 32-bit length can be represented by 30 bit value but needed 3 bits
          // instead 2 bits for metadata, so upper bit is required to be shaved away
          // and limits 32-bit systems to only have maximum 2^31 bytes of array or
          // simply 2 GiB maximum with ~537 millions entries (or 2^29 entries)
          array_length: (word & NON_ORDINARY_DATA_MASK) >> NON_ORDINARY_DATA_SHIFT
        });
      }
      
      unimplemented!();
    }
  }
}


