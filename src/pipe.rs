// A pipe that transfers data both ways instead of atomic and condvars, etc
// I used OS's pipe. Mainly because on some places i cannot wait on userspace
// condvar AND another OS file descrioptor with poll. Necessiting this so can
// be polled. Note: this isnt as performant as even Mutex<VecDeque<T>>. Because
// on fast and slow path are the same => interrrupting to OS

use std::{
    marker::PhantomData,
    mem::{self, MaybeUninit},
    os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd},
    slice,
};

use nix::{
    errno::Errno,
    fcntl::OFlag,
    libc,
    poll::{PollFd, PollFlags, PollTimeout, poll},
    unistd::{pipe2, write},
};

pub struct Pipe<T: Send> {
    read_fd: OwnedFd,
    write_fd: OwnedFd,
    _phantom: PhantomData<T>,
}

impl<T: Send> Drop for Pipe<T> {
    fn drop(&mut self) {
        if !mem::needs_drop::<T>() {
            // Dont need to handle the items already queued in pipe
            // because T doesnt have any drop codes. So do nothing
            // cheaping out on cleanup path. Leaving only constant
            // 2 close() calls
            return;
        }

        // Calls drop code on each items in queue. As long as
        // there data to read
        while let Some(_) = self.read().unwrap() {}
    }
}

impl<T: Send> Pipe<T> {
    pub fn new() -> Result<Self, Errno> {
        let (ro, wr) = pipe2(OFlag::O_CLOEXEC | OFlag::O_DIRECT | OFlag::O_NONBLOCK)?;
        Ok(Self {
            read_fd: ro,
            write_fd: wr,
            _phantom: PhantomData,
        })
    }

    // Returns Ok(Some(...)) if would block (cannot write)
    pub fn write(&self, data: T) -> Result<Option<T>, (Errno, T)> {
        let bytes = unsafe { slice::from_raw_parts(&raw const data as *const u8, size_of::<T>()) };
        match write(self.write_fd.as_fd(), bytes) {
            Ok(x) => {
                // Data is either half written or fully written. Eiher way
                // its not fully owned anymore
                mem::forget(data);
                if x != size_of::<T>() {
                    panic!("We're using O_DIRECT, half write cannot occur")
                }

                Ok(None)
            }

            // NOTE this lint needed because EGAIN and EWOULDBLOCK
            // might be different on some platforms.
            #[expect(unreachable_patterns)]
            Err(Errno::EWOULDBLOCK) | Err(Errno::EAGAIN) => Ok(Some(data)),
            Err(e) => Err((e, data)),
        }
    }

    // Returns Ok(Some(...)) if would block (nothing to be read)
    pub fn read(&self) -> Result<Option<T>, Errno> {
        let mut bytes: MaybeUninit<T> = MaybeUninit::uninit();

        // SAFETY: Destination is valid and sized like T
        let ret = unsafe {
            libc::read(
                self.read_fd.as_raw_fd(),
                bytes.as_mut_ptr().cast(),
                size_of::<T>(),
            )
        };
        if ret == -1 {
            let err = Errno::last();
            if err == Errno::EAGAIN || err == Errno::EWOULDBLOCK {
                // There no data ready to read
                return Ok(None);
            }
            return Err(err);
        }

        if ret != size_of::<T>().try_into().unwrap() {
            panic!("Half reads? or bug on the writer side");
        }

        // SAFETY: We written valid T before. So it is valid. Unless
        // OS is broken
        Ok(Some(unsafe { bytes.assume_init() }))
    }

    pub fn read_blocking(&self) -> Result<T, Errno> {
        loop {
            let mut fds = [PollFd::new(self.write_fd.as_fd(), PollFlags::POLLIN)];
            poll(&mut fds, PollTimeout::NONE)?;

            if !fds[0].revents().unwrap().is_empty() {
                if let Some(data) = self.read()? {
                    return Ok(data);
                }
            }
        }
    }

    pub fn write_blocking(&self, mut data: T) -> Result<(), (Errno, T)> {
        loop {
            let mut fds = [PollFd::new(self.write_fd.as_fd(), PollFlags::POLLOUT)];
            if let Err(e) = poll(&mut fds, PollTimeout::NONE) {
                return Err((e, data));
            }

            if !fds[0].revents().unwrap().is_empty() {
                if let Some(ret_data) = self.write(data)? {
                    // Fail to write, try again
                    data = ret_data;
                } else {
                    // Succesfully written
                    return Ok(());
                }
            }
        }
    }

    pub fn get_read_fd<'a>(&'a self) -> BorrowedFd<'a> {
        self.read_fd.as_fd()
    }

    pub fn get_write_fd<'a>(&'a self) -> BorrowedFd<'a> {
        self.write_fd.as_fd()
    }
}
