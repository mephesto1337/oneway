use std::path::PathBuf;

use tokio::fs::{create_dir_all, symlink_metadata, File};

use crate::Result;

async fn create_directories(filename: &PathBuf) -> Result<()> {
    let parent = filename.parent().unwrap();
    match symlink_metadata(parent).await {
        Ok(metadata) => {
            assert!(metadata.is_dir());
            Ok(())
        }
        Err(e) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                create_dir_all(parent).await?;
                Ok(())
            } else {
                Err(e.into())
            }
        }
    }
}

pub async fn create_file(filename: &PathBuf, size: u64) -> Result<File> {
    create_directories(filename).await?;
    let f = File::create(filename).await?;
    f.set_len(size).await?;

    Ok(f)
}
