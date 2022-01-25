use std::path::PathBuf;

use crate::messages::Message;
use crate::envelope::{EnvelopeHeader, EnvelopeIterator};
use crate::error::{Result, Error};
use crate::config::Config;

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

    async fn send_message(&mut self, message: Message) -> Result<()> {
        let raw_message = bincode::serialize(&message)?;
        let mut envelope_iter = EnvelopeIterator::new(&raw_message[..], &self.config)?;
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

    /// Returns:
    ///   Ok(true) if the read was a success
    ///   Ok(false) if a timeout occured
    ///   Err(_) if trouble
    async fn recv_exact(&mut self, buf: &mut [u8]) -> Result<bool> {
        let sleep = time::sleep(self.config.recv_timeout.clone());
        tokio::pin!(sleep);

        tokio::select! {
            _ = self.reader.read_exact(buf) => {
                return Ok(true)
            }
            _ = &mut sleep => {
                return Ok(false)
            }
        }

    }

    async fn recv_envelope(&mut self) -> Result<(EnvelopeHeader, Vec<u8>)> {
        let envelope_header_size = EnvelopeHeader::size();
        let mut envelopes = Vec::with_capacity(self.config.remission_count);
        let mut envelope_buffer = Vec::with_capacity(envelope_header_size);

        loop {
            if !self.recv_exact(&mut envelope_buffer[..]).await? {
                break;
            }
            let envelope: EnvelopeHeader = bincode::deserialize(&envelope_buffer[..])?;
            let mut data = Vec::with_capacity(envelope.size as usize);
            if !self.recv_exact(&mut data[..]).await? {
                break;
            }

            let should_break = envelope.emission_count == envelope.current_emission;
            envelopes.push((envelope, data));

            if should_break {
                break;
            }
        }

        if let Some((envelope1, data1)) = envelopes.pop() {
            for (_, data) in &envelopes {
                if data != &data1 {
                    log::warn!("Got different data for the same chunk");
                    log::debug!("reference: {:x?}", data1);
                    log::debug!("different: {:x?}", data);
                }
            }
            Ok((envelope1, data1))
        } else {
            Err(Error::NoData)
        }
    }

    pub async fn recv_message(&mut self) -> Result<Message> {
        let mut buffer = Vec::with_capacity(self.config.mtu);

        let (header, data) = self.recv_envelope().await?;
        if header.offset != 0 {
            return Err(Error::MissingData(buffer.len()..header.offset as usize));
        }

        buffer.write(&data[..]).await.expect("Memory write should never fail");
        while (buffer.len() as u64) < header.total_size {
            let (next_header, data) = self.recv_envelope().await?;
            if next_header.offset as usize != buffer.len() {
                return Err(Error::MissingData(buffer.len()..header.offset as usize));
            }
            buffer.write(&data[..]).await.expect("Memory write should never fail");
        }

        let message = bincode::deserialize(&buffer[..])?;

        Ok(message)
    }

}


