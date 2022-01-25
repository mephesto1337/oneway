use serde_derive::{Deserialize, Serialize};

/// Message send from the client to server
#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
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
        created: u64,
        content: Vec<u8>,
    },
}
