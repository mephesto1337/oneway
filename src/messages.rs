use std::io;
use std::mem;

/// Message send from the client to server
#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Message {
    /// Hello message to start a new session
    Hello,

    /// KeepAlive message with an incrementing ID
    KeepAlive(u64),

    /// How many files to upload
    CountFilesToUpload(u64),

    /// A single file been uploaded
    File {
        filename: String,
        size: u64,
        created: u64,
        id: u64,
    },

    /// A Chunk of a file
    FileData { id: u64, offset: u64, data: Vec<u8> },
}

impl Message {
    fn serialize_into(&self, buffer: &mut [u8]) -> io::Result<()> {
        todo!();
    }

    /// Returns a hint on how much space it needs for serializations
    fn size(&self) -> usize {
        let size = match self {
            Self::Hello => 0,
            _ => todo!(),
        };
        size + mem::size_of::<u32>()
    }
}
