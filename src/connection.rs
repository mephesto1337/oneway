use std::path::PathBuf;

use crate::messages::Message;
use crate::error::Result;
use crate::config::Config;

use tokio::io::{AsyncWrite, AsyncWriteExt};

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
        let keep_alive = unsafe { crate::utils::get_random_number().assume_init() };
        Self { writer, config, keep_alive }
    }

    async fn send_message(&mut self, message: Message) -> Result<()> {
        let mut envelope_iter = message.get_envelopes(&self.config)?;
        while let Some(raw_enveloppe) = envelope_iter.get_next_envelope() {
            self.writer.write_all(raw_enveloppe).await?;
        }

        Ok(())
    }

    pub async fn send_hello(&mut self) -> Result<()> {
        let message = Message::Hello;

        self.send_message(message).await
    }

    pub async fn send_keep_alive(&mut self) -> Result<()> {
        let message = Message::KeepAlive(self.keep_alive);
        self.keep_alive = self.keep_alive.wrapping_add(1);

        self.send_message(message).await
    }

    pub async fn send_files(&mut self, files: &[PathBuf]) -> Result<()> {
        let files_count = files.len().try_into()?;

        self.send_message(Message::CountFilesToUpload(files_count)).await?;

        for file in files {
            let content = tokio::fs::read(file).await?;
            let filename = file.to_string_lossy().to_string();
            let created = tokio::fs::symlink_metadata(file).await?.created()?.duration_since(std::time::UNIX_EPOCH)?.as_secs();

            self.send_message(Message::File {filename, content, created}).await?;
        }

        Ok(())
    }
}
