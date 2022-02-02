use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

#[cfg(target_family = "unix")]
fn get_unix_inode(path: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path)?;
    Ok(metadata.ino())
}

#[cfg(target_os = "windows")]
fn get_windows_inode(path: &Path) -> io::Result<u64> {
    use std::mem::MaybeUninit;
    use std::os::windows::io::{AsRawHandle, RawHandle};

    #[allow(non_camel_case_types)]
    #[allow(non_snake_case)]
    #[repr(C)]
    struct BY_HANDLE_FILE_INFORMATION {
        _FileAttributes: u32,
        _CreationTime: u64,
        _LastAccessTime: u64,
        _LastWriteTime: u64,
        _VolumeSerialNumber: u32,
        _FileSizeHigh: u32,
        _FileSizeLow: u32,
        _NumberOfLinks: u32,
        FileIndex: u64,
    }
    type BOOL = u32;
    extern "C" {
        fn GetFileInformationByHandle(
            handle: RawHandle,
            file_information: *mut BY_HANDLE_FILE_INFORMATION,
        ) -> BOOL;
    }

    let file = fs::File::open(path)?;
    let handle = file.as_raw_handle();
    let mut file_information: MaybeUninit<BY_HANDLE_FILE_INFORMATION> = MaybeUninit::uninit();

    // SAFETY: `handle` is a valid file handle and file_information has the right size
    let ret = unsafe { GetFileInformationByHandle(handle, file_information.as_mut_ptr()) };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `GetFileInformationByHandle` returned `TRUE` so `file_information` is initialized
    let file_information = unsafe { file_information.assume_init() };

    Ok(file_information.FileIndex)
}

pub fn get_inode(path: &Path) -> io::Result<u64> {
    #[cfg(target_family = "unix")]
    let inode = get_unix_inode(path)?;

    #[cfg(target_os = "windows")]
    let inode = get_windows_inode(path)?;

    Ok(inode)
}

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
