use std::collections::{hash_map::Entry, HashMap};
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::config::Config;
use crate::messages::Message;
use crate::retransmit::Reassembler;
use crate::udp::UdpReader;
use crate::{Error, Result, Wire};

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

pub struct Server {
    socket: UdpReader,
    config: Config,
    root: PathBuf,
    handlers: HashMap<SocketAddr, ClientHandler>,
}

impl Server {
    pub fn new_with_config(socket: UdpReader, config: Config) -> Self {
        tracing::trace!("Server::new_with_config");
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

    pub async fn recv_message(&mut self) -> Result<()> {
        let mut buffer = vec![0u8; self.config.mtu];
        let (size, client_addr) = self.socket.recv_from(&mut buffer[..]).await?;
        let data = &buffer[..size];

        match self.handlers.entry(client_addr) {
            Entry::Vacant(vac) => {
                log::info!("Creating new handler for {}", vac.key());
                let mut handler = ClientHandler {
                    keep_alive: None,
                    root: self.root.clone(),
                    client_addr: vac.key().clone(),
                    reassembler: Reassembler::new(&self.config),
                    data: Vec::new(),
                };
                handler.process_buffer(data).await;
                vac.insert(handler);
            }
            Entry::Occupied(mut occ) => {
                occ.get_mut().process_buffer(data).await;
            }
        }

        Ok(())
    }

    pub async fn serve_forever(&mut self) -> Result<()> {
        loop {
            self.recv_message().await?;
        }
    }
}

pub struct ClientHandler {
    keep_alive: Option<u64>,
    client_addr: SocketAddr,
    reassembler: Reassembler,
    data: Vec<u8>,
    root: PathBuf,
}

impl ClientHandler {
    pub fn client_addr(&self) -> &SocketAddr {
        &self.client_addr
    }

    async fn process_message_hello(&mut self) {
        tracing::info!("[{}] Received hello from client", self.client_addr());
    }

    async fn process_message_keep_alive(&mut self, id: u64) {
        if let Some(ref mut prev_id) = self.keep_alive {
            let expected_id = prev_id.wrapping_add(1);
            if id != expected_id {
                tracing::warn!(
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
        tracing::info!(
            "[{}] Will received {} files from client",
            self.client_addr(),
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

        let client_addr = self.client_addr().clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            tracing::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        tokio::spawn(async move {
            if let Err(e) = create_file(&real_filename, size).await {
                tracing::error!(
                    "[{}] Could not create file {}: {}",
                    client_addr,
                    real_filename.display(),
                    e
                );
            } else {
                tracing::info!(
                    "[{}] Created file {} of {} bytes",
                    client_addr,
                    real_filename.display(),
                    size
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

        let client_addr = self.client_addr().clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            tracing::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        tokio::spawn(async move {
            let content_size = content.len();
            if let Err(e) = write_chunk_to_file(&real_filename, offset, content).await {
                tracing::error!(
                    "[{}] Could not write chunk at offset 0x{:x} to {:?}: {}",
                    client_addr,
                    offset,
                    real_filename.display(),
                    e
                );
            } else {
                tracing::info!(
                    "[{}] Write {} bytes into {} at {}",
                    client_addr,
                    content_size,
                    real_filename.display(),
                    offset
                );
            }
        });
    }

    async fn process_message_done(&mut self) {
        tracing::info!("[{}] Received done from client", self.client_addr());
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

    async fn process_buffer_internal(&mut self, buffer: &[u8]) -> Result<()> {
        self.reassembler.push_data(buffer);

        match self.reassembler.get_next_data(&mut self.data) {
            Ok(()) => {
                let (rest, message) = Message::from_wire(&self.data[..])?;
                if !rest.is_empty() {
                    tracing::warn!("Got extra data at the end of the message");
                    tracing::warn!("Extra data: {:x?}", rest);
                }
                self.data.clear();
                self.process_message(message).await;
                Ok(())
            }
            Err(Error::NoData) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn process_buffer(&mut self, buffer: &[u8]) {
        if let Err(e) = self.process_buffer_internal(buffer).await {
            tracing::error!("[{}] error: {}", self.client_addr(), e);
        }
    }
}
