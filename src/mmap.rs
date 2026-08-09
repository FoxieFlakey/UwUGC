use std::{ffi::c_void, io, ptr};

use nix::{errno::Errno, libc};

pub struct Mmap {
    ptr: *mut c_void,
    len: usize
}

unsafe impl Sync for Mmap {}
unsafe impl Send for Mmap {}

impl Drop for Mmap {
    fn drop(&mut self) {
        // SAFETY: No, we own the memory
        Errno::result(unsafe { nix::libc::munmap(self.ptr.cast(), self.len) })
            .expect("Cannot unmap temporary mapping");
    }
}

pub enum Advice {
    Remove,
    PopulateRead
}

impl Mmap {
    pub fn get_ptr(&self) -> *mut u8 {
        self.ptr.cast()
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn map(size: usize, is_private: bool, can_read: bool, can_write: bool) -> io::Result<Self> {
        let mut prot = 0;
        if can_read {
            prot |= libc::PROT_READ;
        }

        if can_write {
            prot |= libc::PROT_WRITE;
        }

        if !can_read && !can_write {
            prot |= libc::PROT_NONE;
        }

        let mut flags = libc::MAP_ANON;
        if is_private {
            flags |= libc::MAP_PRIVATE;
        } else {
            flags |= libc::MAP_SHARED;
        }

        let mapped = unsafe { libc::mmap(ptr::null_mut(), size, prot, flags, -1, 0) };
        if mapped == libc::MAP_FAILED {
            Err(nix::errno::Errno::last().into())
        } else {
            // SAFETY: We got it from mmap
            Ok(unsafe { Self::from_raw(mapped.cast(), size) })
        }
    }

    // # Safety
    // The pointer and len give, must corresopnd to valid range
    // mapping from mmap
    pub unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        Self { ptr: ptr.cast(), len }
    }

    // # Safety
    // Some advises is "destructive" like MADV_REMOVE while some dont
    // its up to caller to ensure its safe to do so
    pub unsafe fn advise(&self, advise: Advice) -> io::Result<()> {
        let advice = match advise {
            Advice::PopulateRead => libc::MADV_POPULATE_READ,
            Advice::Remove => libc::MADV_REMOVE,
        };
        
        // SAFETY: bweh
        Errno::result(unsafe { libc::madvise(self.ptr.cast(), self.len, advice) })?;
        Ok(())
    }
}

