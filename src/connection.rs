use std::path::PathBuf;

use crate::messages::Message;
use crate::{Result, Wire};
use crate::config::Config;
use crate::retransmit::Retransmit;

use tokio::io::{AsyncWrite, AsyncWriteExt, AsyncRead, AsyncReadExt, BufReader};
use tokio::time;

pub struct Client<W> {
    writer: W,
    config: Config,
    keep_alive: u64,
}

impl<W> Client<W>
where
    W: AsyncWrite + std::marker::Unpin,
{
    pub fn new(writer: W) -> Self {
        Self::new_with_config(writer, Config::default())
    }

    pub fn new_with_config(writer: W, config: Config) -> Self {
        // SAFETY: any memory representation of a u64 is a valid one
        let keep_alive = unsafe { crate::utils::get_random().assume_init() };
        Self { writer, config, keep_alive }
    }

    async fn send_message(&mut self, message: &Message) -> Result<()> {
        let mut raw_message = Vec::new();
        message.to_wire(&mut raw_message)?;

        let mut retransmit = Retransmit::new(&raw_message[..], self.config.remission_count, self.config.mtu)?;
        retransmit.send(&mut self.writer).await?;

        Ok(())
    }

    pub async fn send_hello(&mut self) -> Result<()> {
        let message = Message::Hello;

        self.send_message(&message).await
    }

    pub async fn send_keep_alive(&mut self) -> Result<()> {
        let message = Message::KeepAlive(self.keep_alive);
        self.keep_alive = self.keep_alive.wrapping_add(1);

        self.send_message(&message).await
    }

    pub async fn send_files(&mut self, files: &[PathBuf]) -> Result<()> {
        let files_count = files.len().try_into()?;

        self.send_message(&Message::CountFilesToUpload(files_count)).await?;

        for file in files {
            let metadata = tokio::fs::symlink_metadata(file).await?;
            let filename = file.to_string_lossy().to_string();
            let created = metadata.created()?;
            let size = metadata.len();

            // First sends the file existance
            self.send_message(&Message::File {filename: filename.clone(), created, size}).await?;

            // Now its content
            let mut f = tokio::fs::File::open(file).await?;
            let mut offset = 064;
            let content = Vec::with_capacity(self.config.mtu - crate::retransmit::max_payload_size(self.config.mtu));
            let mut message = Message::FileChunk { filename, offset, content };
            let buffer = match message {
                Message::FileChunk { ref mut content, .. } => content,
                _ => unreachable!()
            };
            loop {
                buffer.clear();
                let size = f.read(buffer).await?;
            }

        }

        Ok(())
    }
}

pub struct Server<R> {
    reader: BufReader<R>,
    config: Config,
}

impl<R> Server<R>
where
    R: AsyncRead + std::marker::Unpin,
{
    pub fn new(reader: R) -> Self {
        Self::new_with_config(reader, Config::default())
    }

    pub fn new_with_config(reader: R, config: Config) -> Self {
        Self { reader: BufReader::new(reader), config }
    }

    pub async fn recv_message(&mut self) -> Result<Message> {
        let buffer = Retransmit::recv(&mut self.reader, self.config.recv_timeout.clone(), self.config.mtu).await?;

        let (rest, message) = Message::from_wire(&buffer[..])?;
        if ! rest.is_empty() {
            log::warn!("Got extra data from message: {:x?}", rest);
        }

        Ok(message)
    }
}


