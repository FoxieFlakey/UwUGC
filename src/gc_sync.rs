// Implements stuffs like safepoint, STW, etc

use std::{
    cell::UnsafeCell, io, mem, os::fd::AsFd, sync::atomic::{Ordering, fence}
};

use memmap2::{Mmap, MmapOptions, UncheckedAdvice};
use nix::poll::{PollFd, PollFlags, PollTimeout, poll};
use parking_lot::{Mutex, MutexGuard};
use userfaultfd::{Uffd, UffdBuilder};

use crate::pipe::Pipe;

pub struct GCSync<T> {
    lock_page: Mmap,
    inner: UnsafeCell<T>,
    live_threads: Mutex<u32>,
    uffd: Uffd,

    gc_commands: Pipe<Command>,
}

enum Command {
    // Slow path where direct modification to live_threads
    // is not possible. This won't be returned in "wait_command"
    // call
    UnregisterThread,
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
        let thread_activity_queue = match Pipe::new() {
            Ok(x) => x,
            Err(e) => return Err((CreateError::CreatePipe(e), data)),
        };

        match MmapOptions::new().len(page_size::get()).map_anon(true) {
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
                        gc_commands: thread_activity_queue,
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
                PollFd::new(self.gc_commands.get_read_fd(), PollFlags::POLLIN),
                PollFd::new(self.uffd.as_fd(), PollFlags::POLLIN),
            ];

            poll(&mut fds, PollTimeout::NONE).unwrap();

            let thread_activity = fds[0].revents().unwrap();
            let uffd_event = fds[1].revents().unwrap();

            if !thread_activity.is_empty() {
                while let Some(action) = self.gc_commands.read().unwrap() {
                    match action {
                        Command::UnregisterThread => {
                            // A thread is exited. But because this code took lock on live_count.
                            // other thread can't decrement it, thus notifies it here. If live_count
                            // is locked very sure we're going to be here sooner or later
                            *live_count -= 1;
                        }
                    }
                }
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
            _guard: live_count,
        }
    }
}

pub struct ExclusiveGuard<'a, T> {
    owner: &'a GCSync<T>,
    _guard: MutexGuard<'a, u32>,
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
        // SAFETY: We're not getting the &T anymore, at drop code
        unsafe { self.disable_guard() };
    }
}

impl<T> SharedGuard<'_, T> {
    // # Safety
    // enable_guard must be paired with disable_guard!
    unsafe fn enable_guard(&mut self) {
        // Follows what disable_guard wants. mem::forget the disabled guard
        mem::forget(mem::replace(self, self.owner.get_shared()));
    }
    
    // # Safety
    // The guard is disabled, its caller responsibility to never
    // get &T until guard is re-enabled. If needed to drop this
    // guard you must use mem::forget. Or disable_guard is called
    // twice in row, which is not allowed
    unsafe fn disable_guard(&mut self) {
        // Make sure writes cannot happen before this
        // so GC get up to date data.
        fence(Ordering::Release);

        match self.owner.live_threads.try_lock() {
            None => {
                // Notify the writer, that a thread
                // has exited. We cant block here. GC
                // might waits for current thread to
                // safepoint.
                //
                // So go with slow path
                self.owner
                    .gc_commands
                    .write_blocking(Command::UnregisterThread)
                    .map_err(|x| x.0)
                    .unwrap();
            }

            Some(mut live) => {
                *live -= 1;
            }
        }
    }
    
    pub fn get(&self) -> &T {
        // SAFETY: No other thread can access inner mutably
        // as mutable access requires all readers to be blocked
        // on safe page. With existence of SharedGuard. There
        // is thread that is not on safe page
        unsafe { self.owner.inner.get().as_ref_unchecked() }
    }

    // Runs 'f' while having this guard temporarily deactivated
    // &mut make sure no one get &T
    pub fn unguarded<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        // SAFETY: With &mut bound, this function has exclusive
        // reference to self. and the f() has no access to the
        // guard due &mut
        unsafe { self.disable_guard() };

        // Catching unwind is needed because drop code for SharedGuard
        // will call disable_guard which means it will calls it twice
        // So catch it so we can reenable temporarily
        let ret = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));

        // SAFETY: Pairs with disable_guard
        unsafe { self.enable_guard() };

        match ret {
            Ok(ret) => ret,
            Err(e) => std::panic::resume_unwind(e)
        }
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
