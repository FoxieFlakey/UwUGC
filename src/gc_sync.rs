// Implements stuffs like safepoint, STW, etc

use std::{
    cell::UnsafeCell,
    io,
    os::fd::{AsFd, OwnedFd},
    sync::atomic::{Ordering, fence},
};

use memmap2::{Mmap, MmapOptions, UncheckedAdvice};
use nix::{
    fcntl::OFlag,
    poll::{PollFd, PollFlags, PollTimeout, poll},
};
use parking_lot::{Mutex, MutexGuard};
use userfaultfd::{Uffd, UffdBuilder};

pub struct GCSync<T> {
    lock_page: Mmap,
    inner: UnsafeCell<T>,
    live_threads: Mutex<u32>,
    uffd: Uffd,

    // thread wanting to unregister writes to this
    unregister_write_fd: OwnedFd,

    // thread wants to receive unregister reads this
    unregister_read_fd: OwnedFd,
}

unsafe impl<T: Sync> Sync for GCSync<T> {}
unsafe impl<T: Send> Send for GCSync<T> {}

#[derive(thiserror::Error, Debug)]
pub enum CreateError {
    #[error("cannot map lock page")]
    MappingLockPage(io::Error),
    #[error("cannot change lock page to be read only")]
    ChangeToRO(io::Error),
    #[error("cannot init UFFD")]
    InitUFFD(userfaultfd::Error),
    #[error("cannot create pipe")]
    CreatePipe(nix::errno::Errno),
}

impl<T> GCSync<T> {
    pub fn new(data: T) -> Result<Self, (CreateError, T)> {
        let (ro, wr) = match nix::unistd::pipe2(
            OFlag::O_RDWR | OFlag::O_CLOEXEC | OFlag::O_DIRECT | OFlag::O_NONBLOCK,
        ) {
            Ok(x) => x,
            Err(e) => return Err((CreateError::CreatePipe(e), data)),
        };

        match MmapOptions::new().len(page_size::get()).map_anon() {
            Ok(lock_page) => match lock_page.make_read_only() {
                Ok(lock_page) => match UffdBuilder::new()
                    .non_blocking(true)
                    .close_on_exec(true)
                    .create()
                {
                    Ok(uffd) => Ok(Self {
                        live_threads: Mutex::new(0),
                        inner: UnsafeCell::new(data),
                        lock_page,
                        uffd,
                        unregister_read_fd: ro,
                        unregister_write_fd: wr,
                    }),

                    Err(e) => Err((CreateError::InitUFFD(e), data)),
                },

                Err(e) => Err((CreateError::ChangeToRO(e), data)),
            },

            Err(e) => Err((CreateError::MappingLockPage(e), data)),
        }
    }

    // This does heavy lock on live_threads counter. Preferably save the
    // SharedGuard in per thread data and use guard.safepoint() for places
    // where writer/GC can blocks current thread. Instead of dropping the
    // instance or recreate it everytime
    pub fn get_shared<'a>(&'a self) -> SharedGuard<'a, T> {
        *self.live_threads.lock() += 1;

        // Fence necessary so current thread dont read data
        // before its part of live threads because GC/writer
        // might already writes data before the lock
        fence(Ordering::Acquire);

        SharedGuard { owner: self }
    }

    pub fn get_exclusive<'a>(&'a self) -> ExclusiveGuard<'a, T> {
        let mut live_count = self.live_threads.lock();

        // The content of it doesn't matter
        unsafe {
            self.lock_page
                .unchecked_advise(UncheckedAdvice::Remove)
                .unwrap()
        };

        let mut blocked_count = 0;
        while blocked_count < *live_count {
            let mut fds = [
                PollFd::new(self.unregister_read_fd.as_fd(), PollFlags::POLLIN),
                PollFd::new(self.uffd.as_fd(), PollFlags::POLLIN),
            ];

            poll(&mut fds, PollTimeout::NONE).unwrap();

            let unregister_event = fds[0].revents().unwrap();
            let uffd_event = fds[1].revents().unwrap();

            if !unregister_event.is_empty() {
                // A thread is exited. But because this code took lock on live_count.
                // other thread can't decrement it, thus notifies it here. If live_count
                // is locked very sure we're going to be here sooner or later
                *live_count -= 1;
            }

            if !uffd_event.is_empty() {
                while let Some(event) = self.uffd.read_event().unwrap() {
                    match event {
                        userfaultfd::Event::Pagefault { .. } => {
                            // One thread is blocked now
                            blocked_count += 1;
                        }

                        _ => unimplemented!(),
                    }
                }
            }
        }
        // Make sure GC/writer dont read data before all threads are parked.
        // which might get stale
        fence(Ordering::Acquire);

        ExclusiveGuard {
            owner: self,
            guard: live_count,
        }
    }
}

pub struct ExclusiveGuard<'a, T> {
    owner: &'a GCSync<T>,
    guard: MutexGuard<'a, u32>,
}

impl<T> ExclusiveGuard<'_, T> {
    pub fn get(&mut self) -> &mut T {
        // SAFETY: With existence of this, we already checked that
        // no other thread is running concurrently
        unsafe { self.owner.inner.get().as_mut_unchecked() }
    }
}

impl<T> Drop for ExclusiveGuard<'_, T> {
    fn drop(&mut self) {
        // Fence needed so changes by GC/writer will be visible to
        // mutator before its waken up. This also prevents GC/writer
        // from writing after the wakeup so mutators dont get stale
        // data
        fence(Ordering::Release);

        let uffd = &self.owner.uffd;
        unsafe {
            uffd.zeropage(
                self.owner.lock_page.as_ptr().cast_mut().cast(),
                page_size::get(),
                true,
            )
        }
        .unwrap();

        // the guard would be dropped later
    }
}

pub struct SharedGuard<'a, T> {
    owner: &'a GCSync<T>,
}

impl<T> Drop for SharedGuard<'_, T> {
    fn drop(&mut self) {
        // Make sure writes cannot happen before this
        // so GC get up to date data.
        fence(Ordering::Release);
        
        match self.owner.live_threads.try_lock() {
            None => {
                // Notify the writer, that a thread
                // has exited. We cant block here. GC
                // might waits for current thread to
                // safepoint
                nix::unistd::write(&self.owner.unregister_write_fd, &[0]).unwrap();
            }

            Some(mut live) => {
                *live -= 1;
            }
        }
    }
}

impl<T> SharedGuard<'_, T> {
    pub fn get(&self) -> &T {
        // SAFETY: No other thread can access inner mutably
        // as mutable access requires all readers to be blocked
        // on safe page. With existence of SharedGuard. There
        // is thread that is not on safe page
        unsafe { self.owner.inner.get().as_ref_unchecked() }
    }

    pub fn safepoint(&self) {
        // Release fence necessary because writer/GC might access
        // stale data. if compiler writes it after the safepoint
        fence(Ordering::Release);

        // Triggers safepoint, to maybe blocks if writer is waiting
        // SAFETY: We dont do anything than read and discard...
        unsafe { self.owner.lock_page.as_ptr().read_volatile() };

        // Fence necessary so current thread dont read data
        // before the safepoint because it can get out of date
        fence(Ordering::Acquire);
    }
}
