use std::collections::HashMap;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::Config;
use crate::messages::Message;
use crate::retransmit::Retransmit;
use crate::udp::UdpReader;
use crate::{Error, Result, Wire};

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub struct Server {
    socket: UdpReader,
    config: Config,
    root: PathBuf,
    handlers: HashMap<SocketAddr, ServerHandler>,
}

impl Server {
    pub fn new_with_config(socket: UdpReader, config: Config) -> Self {
        log::trace!("Server::new_with_config");
        Self {
            socket,
            config,
            root: std::env::current_dir().expect("Cannot get current directory"),
            handlers: HashMap::new(),
        }
    }

    pub fn set_root(&mut self, root: impl AsRef<Path>) {
        self.root = root.as_ref().to_path_buf();
    }

    pub async fn recv_message(&mut self) -> Result<Message> {
        let buffer = Retransmit::recv(
            &self.socket,
            self.config.recv_timeout.clone(),
            self.config.mtu,
        )
        .await?;

        let (rest, message) = Message::from_wire(&buffer[..])?;
        if !rest.is_empty() {
            log::warn!(
                "[{}] Got extra data from message: {:x?}",
                self.client_addr,
                rest
            );
        }

        Ok(message)
    }

    pub async fn serve_forever(&mut self) -> Result<()> {
        loop {
            log::trace!("Waiting for a message");
            let message = self.recv_message().await?;
            log::debug!("Received message: {:?}", message);
            if matches!(message, Message::Done) {
                break;
            } else {
                self.process_message(message).await;
            }
        }
        Ok(())
    }
}

pub struct ServerHandler {
    keep_alive: Option<u64>,
    client_addr: SocketAddr,
    root: PathBuf,
}

impl ServerHandler {
    pub fn client_addr(&self) -> &SocketAddr {
        &self.client_addr
    }

    async fn process_message_hello(&mut self) {
        log::info!("[{}] Received hello from client", self.client_addr);
    }

    async fn process_message_keep_alive(&mut self, id: u64) {
        if let Some(ref mut prev_id) = self.keep_alive {
            let expected_id = prev_id.wrapping_add(1);
            if id != expected_id {
                log::warn!(
                    "[{}] Got bad keep alive id, expected: {}, got: {}",
                    self.client_addr,
                    expected_id,
                    id
                );
            }
            *prev_id = id;
        } else {
            self.keep_alive = Some(id);
        }
    }

    async fn process_message_count_files_to_upload(&self, count: u64) {
        log::info!(
            "[{}] Will received {} files from client",
            self.client_addr,
            count
        );
    }

    async fn process_message_file(&self, filename: String, _created: SystemTime, size: u64) {
        async fn create_directories(filename: &PathBuf) -> Result<()> {
            let parent = filename.parent().unwrap();
            match tokio::fs::symlink_metadata(parent).await {
                Ok(metadata) => {
                    assert!(metadata.is_dir());
                    Ok(())
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        tokio::fs::create_dir_all(parent).await?;
                        Ok(())
                    } else {
                        Err(e.into())
                    }
                }
            }
        }

        async fn create_file(filename: &PathBuf, size: u64) -> Result<()> {
            create_directories(filename).await?;
            let f = File::create(filename).await?;
            f.set_len(size).await?;

            Ok::<(), Error>(())
        }

        let client_addr = self.client_addr.clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            log::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        tokio::spawn(async move {
            if let Err(e) = create_file(&real_filename, size).await {
                log::error!(
                    "[{}] Could not create file {}: {}",
                    client_addr,
                    real_filename.display(),
                    e
                );
            }
        });
    }

    async fn process_message_file_chunk(&self, filename: String, offset: u64, content: Vec<u8>) {
        async fn write_chunk_to_file(
            filename: &PathBuf,
            offset: u64,
            content: Vec<u8>,
        ) -> Result<()> {
            let mut f = OpenOptions::new().write(true).open(filename).await?;
            f.seek(SeekFrom::Start(offset)).await?;
            f.write_all(&content[..]).await?;

            Ok(())
        }

        let client_addr = self.client_addr.clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            log::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        //tokio::spawn(async move {
        if let Err(e) = write_chunk_to_file(&real_filename, offset, content).await {
            log::error!(
                "[{}] Could not write chunk at offset 0x{:x} to {:?}: {}",
                client_addr,
                offset,
                real_filename.display(),
                e
            );
        }
        //});
    }

    async fn process_message_done(&mut self) {
        log::info!("[{}] Received done from client", self.client_addr);
    }

    pub async fn process_message(&mut self, message: Message) {
        match message {
            Message::Hello => self.process_message_hello().await,
            Message::KeepAlive(id) => self.process_message_keep_alive(id).await,
            Message::CountFilesToUpload(count) => {
                self.process_message_count_files_to_upload(count).await
            }
            Message::File {
                filename,
                created,
                size,
            } => self.process_message_file(filename, created, size).await,
            Message::FileChunk {
                filename,
                offset,
                content,
            } => {
                self.process_message_file_chunk(filename, offset, content)
                    .await
            }
            Message::Done => self.process_message_done().await,
        }
    }
}
