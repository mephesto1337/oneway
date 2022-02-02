use std::fmt;

pub(crate) enum Shutdown {
    Read,
    Write,
}

#[cfg(target_family = "unix")]
pub(crate) use unix::{get_random, shutdown};

#[cfg(target_os = "windows")]
pub(crate) use windows::{get_random, shutdown};

#[cfg(target_family = "unix")]
mod unix {
    use super::Shutdown;
    use std::io;
    use std::io::Read;
    use std::mem::MaybeUninit;
    use std::os::unix::io::AsRawFd;

    pub(crate) fn get_random<T>() -> MaybeUninit<T> {
        let mut file =
            std::fs::File::open("/dev/urandom").expect("No /dev/urandom on an UNIX system?!");
        let mut value: MaybeUninit<T> = MaybeUninit::zeroed();

        // SAFETY: we own the memory
        let slice = unsafe {
            std::slice::from_raw_parts_mut(value.as_mut_ptr().cast(), std::mem::size_of::<T>())
        };

        file.read(slice)
            .expect("Could not read from /dev/urandom?!");

        value
    }

    pub(crate) fn shutdown(socket: &impl AsRawFd, how: Shutdown) -> io::Result<()> {
        let fd = socket.as_raw_fd();
        let how = match how {
            Shutdown::Read => libc::SHUT_RD,
            Shutdown::Write => libc::SHUT_WR,
        };

        let ret = unsafe { libc::shutdown(fd, how) };
        if ret < 0 {
            Err(io::Error::last_os_error().into())
        } else {
            Ok(())
        }
    }
}

#[cfg(target_os = "windows")]
mod windows {
    use super::Shutdown;
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::windows::io::AsRawSocket;

    mod ws2 {
        use std::os::windows::raw::SOCKET;
        #[link(name = "Ws2_32")]
        extern "C" {
            pub fn shutdown(s: SOCKET, how: i32) -> i32;
        }
    }

    pub(crate) fn shutdown(socket: &impl AsRawSocket, how: Shutdown) -> io::Result<()> {
        const SD_RECEIVE: i32 = 0;
        const SD_SEND: i32 = 1;
        let fd = socket.as_raw_socket();
        let how = match how {
            Shutdown::Read => SD_RECEIVE,
            Shutdown::Write => SD_SEND,
        };

        let ret = unsafe { ws2::shutdown(fd, how) };
        if ret == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error().into())
        }
    }

    pub(crate) fn get_random<T>() -> MaybeUninit<T> {
        let value: MaybeUninit<T> = MaybeUninit::uninit();
        value
    }
}

pub struct Hex<'a>(&'a [u8]);

impl<'a> Hex<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self(data)
    }
}

impl fmt::Debug for Hex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("\"")?;
        for byte in self.0.iter() {
            write!(f, "{:02x}", byte)?;
        }
        f.write_str("\"")?;
        Ok(())
    }
}
