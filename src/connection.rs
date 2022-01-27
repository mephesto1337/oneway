use std::path::PathBuf;

use crate::config::Config;
use crate::messages::Message;
use crate::retransmit::Retransmit;
use crate::udp::UdpWriter;
use crate::{Result, Wire};

use tokio::io::{AsyncReadExt, AsyncSeekExt};

mod server;
pub use server::Server;

pub struct Client {
    socket: UdpWriter,
    config: Config,
    keep_alive: u64,
}

impl Client {
    pub fn new(socket: UdpWriter) -> Self {
        Self::new_with_config(socket, Config::default())
    }

    pub fn new_with_config(socket: UdpWriter, config: Config) -> Self {
        // SAFETY: any memory representation of a u64 is a valid one
        let keep_alive = unsafe { crate::utils::get_random().assume_init() };
        Self {
            socket,
            config,
            keep_alive,
        }
    }

    async fn send_message(&mut self, message: &Message) -> Result<()> {
        let mut raw_message = Vec::new();
        log::debug!("Sending message: {:?}", message);
        message.to_wire(&mut raw_message)?;

        let mut retransmit = Retransmit::new(
            &raw_message[..],
            self.config.remission_count,
            self.config.mtu,
        )?;
        retransmit.send(&self.socket).await?;

        Ok(())
    }

    pub async fn send_hello(&mut self) -> Result<()> {
        let message = Message::Hello;

        self.send_message(&message).await?;
        log::info!("Send Hello to server");
        Ok(())
    }

    pub async fn send_keep_alive(&mut self) -> Result<()> {
        let keep_alive = self.keep_alive;
        let message = Message::KeepAlive(self.keep_alive);
        self.keep_alive = self.keep_alive.wrapping_add(1);

        self.send_message(&message).await?;
        log::debug!("Send keep alive ({}) to server", keep_alive);
        Ok(())
    }

    async fn send_file(&mut self, file: &PathBuf) -> Result<()> {
        let metadata = tokio::fs::symlink_metadata(file).await?;
        let filename = file.to_string_lossy().to_string();
        let created = metadata.created()?;
        let size = metadata.len();

        // First sends the file existance
        self.send_message(&Message::File {
            filename: filename.clone(),
            created,
            size,
        })
        .await?;
        log::debug!("Notify server of file {}", filename);

        // Now its content
        let mut f = tokio::fs::File::open(file).await?;
        let content_size = crate::retransmit::max_payload_size(self.config.mtu);
        let content = vec![0u8; content_size];
        let mut message = Message::FileChunk {
            filename,
            offset: 0,
            content,
        };

        loop {
            match message {
                Message::FileChunk {
                    ref mut offset,
                    ref mut content,
                    ref filename,
                } => {
                    *offset = f.stream_position().await?;
                    content.resize(content_size, 0);
                    let size = f.read(&mut content[..]).await?;
                    content.truncate(size);
                    if size == 0 {
                        log::info!("File {} sent to server ({} bytes)", filename, *offset);
                        break;
                    }
                }
                _ => unreachable!(),
            }
            self.send_message(&message).await?;
        }

        Ok(())
    }

    pub async fn send_files(&mut self, files: &[PathBuf]) -> Result<()> {
        let files_count = files.len().try_into()?;

        self.send_message(&Message::CountFilesToUpload(files_count))
            .await?;

        for file in files {
            self.send_file(file).await?;
        }

        Ok(())
    }

    pub async fn send_done(&mut self) -> Result<()> {
        let message = Message::Done;

        self.send_message(&message).await?;
        log::info!("Send Done to server");
        Ok(())
    }
}
