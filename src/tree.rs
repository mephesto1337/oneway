use std::collections::{HashSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::utils::get_inode;

macro_rules! try_with_message {
    ($e:expr => $goto:tt, $fmt:expr, $($args:expr),*) => {{
        match $e {
            Ok(v) => v,
            Err(e) => {
                log::warn!($fmt, $($args),*, e = e);
                continue $goto;
            }
        }
    }};

    ($e:expr => $fmt:expr, $($args:expr),*) => {{
        match $e {
            Ok(v) => v,
            Err(e) => {
                log::warn!($fmt, $($args),*, e = e);
                continue;
            }
        }
    }};

}

pub fn find_files(
    root: impl AsRef<Path>,
    follow_symlinks: bool,
    filter: impl Fn(&Path) -> bool,
) -> Result<Vec<PathBuf>> {
    let mut entries = Vec::new();
    let mut collected_inodes = HashSet::new();
    let mut directories_to_visit = VecDeque::new();
    let root = if root.as_ref().is_absolute() {
        root.as_ref().to_path_buf()
    } else {
        let cwd = std::env::current_dir()?;
        cwd.join(root.as_ref()).canonicalize()?
    };
    directories_to_visit.push_back(root.clone());

    while let Some(dir) = directories_to_visit.pop_front() {
        let dir_entries =
            try_with_message!(dir.read_dir() => "Could not read directory {}: {e}", dir.display());

        'next_entry: for entry in dir_entries {
            let entry = try_with_message!(entry => 'next_entry, "Could not retrieve entry from directory {}: {e}",
                                        dir.display());

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
                        if v.is_absolute() {
                            v
                        } else {
                            let parent = current_entry.parent().unwrap();
                            parent.join(v).canonicalize()?
                        }
                    }
                    Err(e) => {
                        log::warn!("Could not read symlink {}: {}", current_entry.display(), e);
                        continue 'next_entry;
                    }
                };
                metadata = try_with_message!(
                    fs::symlink_metadata(&current_entry) =>
                    "Could not retrieve metadata for {}: {e}",
                    current_entry.display()
                );
            }

            let inode = try_with_message!(get_inode(&current_entry) => "Could not get inode for {}: {}", current_entry.display());
            let entry_is_already_processed = !collected_inodes.insert(inode);

            if entry_is_already_processed {
                log::debug!(
                    "Skipping {} as it has already been visited",
                    current_entry.display()
                );
                continue 'next_entry;
            }

            if metadata.is_dir() {
                directories_to_visit.push_back(current_entry);
                continue 'next_entry;
            }

            if metadata.is_file() {
                if filter(&current_entry) {
                    if let Ok(relative_entry) = current_entry.strip_prefix(&root) {
                        entries.push(relative_entry.to_path_buf());
                    } else {
                        log::warn!(
                            "{} is not in {}, skipping",
                            current_entry.display(),
                            root.display()
                        );
                    }
                }
                continue 'next_entry;
            }
        }
    }

    Ok(entries)
}
