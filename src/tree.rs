use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[cfg(target_family = "unix")]
fn get_unix_inode(path: &Path) -> io::Result<u64> {
    use std::os::unix::fs::MetadataExt;

    let metadata = fs::metadata(path)?;
    Ok(metadata.ino())
}

#[cfg(target_os = "windows")]
fn get_windows_inode(path: &Path) -> io::Result<u64> {
    use std::os::windows::io::{AsRawHandle, RawHandle};

    #[allow(non_camel_case_types)]
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
        ) -> u32;
    }

    let file = fs::File::open(path)?;
    let handle = file.as_raw_handle();
    let mut file_information = std::mem::MaybeUninit::uninit::<BY_HANDLE_FILE_INFORMATION>();

    // SAFETY: `handle` is a valid file handle and file_information has the right size
    let ret = unsafe { GetFileInformationByHandle(handle, file_information.as_mut_ptr()) };
    if ret == 0 {
        return Err(io::Error::last_os_error());
    }

    // SAFETY: `GetFileInformationByHandle` returned `TRUE` so `file_information` is initialized
    let file_information = unsafe { file_information.assume_init() };

    Ok(file_information.FileIndex)
}

fn get_inode(path: &Path) -> io::Result<u64> {
    #[cfg(target_family = "unix")]
    let inode = get_unix_inode(path)?;

    #[cfg(target_os = "windows")]
    let inode = get_windows_inode(path)?;

    Ok(inode)
}

pub fn find_files(
    root: impl AsRef<Path>,
    follow_symlinks: bool,
    filter: impl Fn(&Path) -> bool,
) -> Vec<PathBuf> {
    let mut entries = Vec::new();
    let mut collected_inodes = HashSet::new();
    let mut directories_to_visit = VecDeque::new();
    directories_to_visit.push_back(root.as_ref().to_path_buf());

    while let Some(dir) = directories_to_visit.pop_front() {
        let dir_entries = match dir.read_dir() {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Could not read directory {}: {}", dir.display(), e);
                continue;
            }
        };

        'next_entry: for entry in dir_entries {
            let entry = match entry {
                Ok(v) => v,
                Err(e) => {
                    log::warn!(
                        "Could not retrieve entry from directory {}: {}",
                        dir.display(),
                        e
                    );
                    continue 'next_entry;
                }
            };

            let mut current_entry = entry.path();
            let mut metadata = match entry.metadata() {
                Ok(v) => v,
                Err(e) => {
                    log::warn!(
                        "Could not retrieve metadata for {}: {}",
                        current_entry.display(),
                        e
                    );
                    continue 'next_entry;
                }
            };

            while follow_symlinks && metadata.is_symlink() {
                current_entry = match fs::read_link(&current_entry) {
                    Ok(v) => {
                        log::debug!("Read link {} -> {}", current_entry.display(), v.display());
                        v
                    }
                    Err(e) => {
                        log::warn!("Could not read symlink {}: {}", current_entry.display(), e);
                        continue 'next_entry;
                    }
                };
                metadata = match fs::symlink_metadata(&current_entry) {
                    Ok(v) => v,
                    Err(e) => {
                        log::warn!(
                            "Could not retrieve metadata for {}: {}",
                            current_entry.display(),
                            e
                        );
                        continue 'next_entry;
                    }
                };
            }

            let inode = match get_inode(&current_entry) {
                Ok(v) => v,
                Err(e) => {
                    log::warn!("Could not get inode for {}: {}", current_entry.display(), e);
                    continue 'next_entry;
                }
            };
            let entry_is_already_processed = !collected_inodes.insert(inode);

            if entry_is_already_processed {
                continue 'next_entry;
            }

            if metadata.is_dir() {
                directories_to_visit.push_back(current_entry);
                continue 'next_entry;
            }

            if metadata.is_file() {
                if filter(&current_entry) {
                    entries.push(current_entry);
                }
                continue 'next_entry;
            }
        }
    }

    entries
}
