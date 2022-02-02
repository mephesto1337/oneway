use crate::{Result, Wire};

use std::fmt;
use std::mem::{size_of, size_of_val};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use nom::bytes::streaming::take;
use nom::combinator::{map, map_opt, map_res};
use nom::error::context;
use nom::number::streaming::{be_u16, be_u64, be_u8};

/// Message send from the client to server
#[derive(PartialEq, Eq)]
pub enum Message {
    /// Hello message to start a new session
    Hello,

    /// KeepAlive message with an incrementing ID
    KeepAlive(u64),

    /// How many files to upload
    CountFilesToUpload(u64),

    /// A single file to crate
    File {
        filename: String,
        created: SystemTime,
        size: u64,
        id: u64,
    },

    /// A chunk of data from a file
    /// Files are sent from chunks if we want to transmit a verry large file that would involve
    /// reassemble of all packets and store them in memory before writing.
    FileChunk {
        id: u64,
        offset: u64,
        content_size: u16,
        content: Vec<u8>,
    },

    /// Client is done
    Done,
}

impl Message {
    pub const fn get_max_content_size(mtu: usize) -> usize {
        let mut prefix_size = size_of::<u8>(); // MesageKind
        prefix_size += size_of::<u64>(); // filename id
        prefix_size += size_of::<u64>(); // offset
        prefix_size += size_of::<u16>(); // content_size
        mtu - prefix_size
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Hello => write!(f, "Hello"),
            Self::KeepAlive(ka) => f.debug_tuple("KeepAlive").field(ka).finish(),
            Self::CountFilesToUpload(count) => {
                f.debug_tuple("CountFilesToUpload").field(count).finish()
            }
            Self::File {
                filename,
                created,
                size,
                id,
            } => f
                .debug_struct("File")
                .field("filename", filename)
                .field("created", created)
                .field("size", size)
                .field("id", id)
                .finish(),
            Self::FileChunk {
                id,
                offset,
                content_size,
                content,
            } => f
                .debug_struct("FileChunk")
                .field("id", id)
                .field("offset", offset)
                .field("content_size", content_size)
                .field(
                    "content",
                    &crate::utils::Hex::new(&content[..*content_size as usize]),
                )
                .finish(),
            Self::Done => write!(f, "Done"),
        }
    }
}

#[repr(u8)]
#[derive(Copy, Clone)]
enum MessageKind {
    Hello,
    KeepAlive,
    CountFilesToUpload,
    File,
    FileChunk,
    Done,
}

impl MessageKind {
    fn from_u8(mk: u8) -> Option<Self> {
        match mk {
            0 => Some(Self::Hello),
            1 => Some(Self::KeepAlive),
            2 => Some(Self::CountFilesToUpload),
            3 => Some(Self::File),
            4 => Some(Self::FileChunk),
            5 => Some(Self::Done),
            _ => None,
        }
    }

    fn to_u8(&self) -> u8 {
        *self as u8
    }
}

impl Wire<'_> for Message {
    fn from_wire(input: &[u8]) -> Result<(&[u8], Self)> {
        let (rest, message_kind) =
            context("Message/kind", map_opt(be_u8, MessageKind::from_u8))(input)?;
        match message_kind {
            MessageKind::Hello => Ok((rest, Self::Hello)),
            MessageKind::KeepAlive => {
                let (rest, id) = context("Message/KeepAlive/id", be_u64)(rest)?;
                Ok((rest, Message::KeepAlive(id)))
            }
            MessageKind::CountFilesToUpload => {
                let (rest, count) = context("Message/CountFilesToUpload/count", be_u64)(rest)?;
                Ok((rest, Message::CountFilesToUpload(count)))
            }
            MessageKind::File => {
                let (rest, filename_len) = context("Message/File/filename_len", be_u16)(rest)?;
                let (rest, filename) = context(
                    "Message/File/filename",
                    map(
                        map_res(take(filename_len), std::str::from_utf8),
                        String::from,
                    ),
                )(rest)?;

                let (rest, created) = context(
                    "Message/File/created",
                    map_opt(be_u64, |offset| {
                        UNIX_EPOCH.checked_add(Duration::from_secs(offset))
                    }),
                )(rest)?;

                let (rest, size) = context("Message/File/size", be_u64)(rest)?;

                let (rest, id) = context("Message/File/id", be_u64)(rest)?;

                Ok((
                    rest,
                    Self::File {
                        filename,
                        created,
                        size,
                        id,
                    },
                ))
            }
            MessageKind::FileChunk => {
                let (rest, id) = context("Message/FileChunk/id", be_u64)(rest)?;

                let (rest, offset) = context("Message/FileChunk/offeet", be_u64)(rest)?;

                let (rest, content_size) = context("Message/FileChunk/content_size", be_u16)(rest)?;
                let (rest, content) = context(
                    "Message/FileChunk/content",
                    map(take(content_size as usize), |slice: &[u8]| slice.to_vec()),
                )(rest)?;
                Ok((
                    rest,
                    Self::FileChunk {
                        id,
                        offset,
                        content_size,
                        content,
                    },
                ))
            }
            MessageKind::Done => Ok((rest, Self::Done)),
        }
    }

    fn to_wire<W>(&self, mut writer: W) -> Result<usize>
    where
        W: std::io::Write,
    {
        let mut total_size = 0;
        match self {
            Self::Hello => {
                let mk = MessageKind::Hello.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;
            }
            Self::KeepAlive(ref id) => {
                let mk = MessageKind::KeepAlive.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;

                total_size += size_of_val(id);
                writer.write_all(&id.to_be_bytes()[..])?;
            }
            Self::CountFilesToUpload(ref count) => {
                let mk = MessageKind::CountFilesToUpload.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;
                total_size += size_of_val(count);
                writer.write_all(&count.to_be_bytes()[..])?;
            }
            Self::File {
                ref filename,
                ref created,
                ref size,
                ref id,
            } => {
                let mk = MessageKind::File.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;

                let filename_len: u16 = filename.len().try_into()?;
                total_size += size_of_val(&filename_len);
                writer.write_all(&filename_len.to_be_bytes()[..])?;

                total_size += filename.as_bytes().len();
                writer.write_all(filename.as_bytes())?;

                let offset = created.duration_since(UNIX_EPOCH)?.as_secs();
                total_size += size_of_val(&offset);
                writer.write_all(&offset.to_be_bytes()[..])?;

                total_size += size_of_val(size);
                writer.write_all(&size.to_be_bytes()[..])?;

                total_size += size_of_val(id);
                writer.write_all(&id.to_be_bytes()[..])?;
            }
            Self::FileChunk {
                ref id,
                ref offset,
                ref content_size,
                ref content,
            } => {
                let mk = MessageKind::FileChunk.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;

                total_size += size_of_val(id);
                writer.write_all(&id.to_be_bytes()[..])?;

                total_size += size_of_val(offset);
                writer.write_all(&offset.to_be_bytes()[..])?;

                total_size += size_of_val(content_size);
                writer.write_all(&content_size.to_be_bytes()[..])?;

                let buffer = &content[..*content_size as usize];
                total_size += buffer.len();
                writer.write_all(&buffer[..])?;
            }
            Self::Done => {
                let mk = MessageKind::Done.to_u8();
                total_size += size_of_val(&mk);
                writer.write_all(&[mk])?;
            }
        }

        Ok(total_size)
    }
}
