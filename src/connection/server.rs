use std::collections::{hash_map::Entry, HashMap};
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::SystemTime;

use crate::config::Config;
use crate::messages::Message;
use crate::retransmit::Reassembler;
use crate::udp::UdpReader;
use crate::{Error, Result, Wire};

use tokio::fs::File;
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
        let config_root = PathBuf::from(&config.root);
        let root = if config_root.is_absolute() {
            config_root
        } else {
            let cwd = std::env::current_dir().expect("Cannot get current directory");
            cwd.join(config_root)
        };

        Self {
            socket,
            config,
            root,
            handlers: HashMap::new(),
        }
    }

    pub async fn recv_message(&mut self) -> Result<()> {
        let mut buffer = vec![0u8; self.config.mtu];
        let (size, client_addr) = self.socket.recv_from(&mut buffer[..]).await?;
        let data = &buffer[..size];
        let key = client_addr.clone();

        let done = match self.handlers.entry(client_addr) {
            Entry::Vacant(vac) => {
                log::info!("Creating new handler for {}", vac.key());
                let mut handler = ClientHandler {
                    keep_alive: None,
                    root: self.root.clone(),
                    client_addr: vac.key().clone(),
                    reassembler: Reassembler::new(&self.config),
                    data: Vec::new(),
                    opened_files: HashMap::new(),
                };
                let done = handler.process_buffer(data).await;
                vac.insert(handler);
                done
            }
            Entry::Occupied(mut occ) => occ.get_mut().process_buffer(data).await,
        };

        if done {
            log::info!("Removing handler for {}", &key);
            self.handlers.remove(&key);
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
    opened_files: HashMap<u64, File>,
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

    async fn process_message_file(
        &mut self,
        filename: String,
        _created: SystemTime,
        size: u64,
        id: u64,
    ) {
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

        async fn create_file(filename: &PathBuf, size: u64) -> Result<File> {
            create_directories(filename).await?;
            let f = File::create(filename).await?;
            f.set_len(size).await?;

            Ok(f)
        }

        let client_addr = self.client_addr().clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            tracing::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        // tokio::spawn(async move {
        match create_file(&real_filename, size).await {
            Ok(f) => {
                tracing::info!(
                    "[{}] Created file {} of {} bytes (id: 0x{:x})",
                    client_addr,
                    real_filename.display(),
                    size,
                    id
                );
                self.opened_files.insert(id, f);
            }
            Err(e) => {
                tracing::error!(
                    "[{}] Could not create file {}: {}",
                    client_addr,
                    real_filename.display(),
                    e
                );
            }
        }
        // });
    }

    async fn process_message_file_chunk(
        &mut self,
        id: u64,
        offset: u64,
        content_size: u16,
        content: Vec<u8>,
    ) {
        async fn write_chunk_to_file(file: &mut File, offset: u64, content: &[u8]) -> Result<()> {
            if content.iter().all(|x| *x == 0) {
                log::warn!("Go all zero chunk at {}", offset);
            }
            file.seek(SeekFrom::Start(offset)).await?;
            file.write_all(content).await?;
            // f.flush().await?;

            Ok(())
        }

        // If content_size is 0, then the file has been sent
        if content_size == 0 {
            tracing::info!("Done receiving 0x{:x}", id);
            self.opened_files.remove(&id);
            return;
        }

        let client_addr = self.client_addr().clone();
        // tokio::spawn(async move {
        let buffer = &content[..content_size as usize];
        let f = match self.opened_files.get_mut(&id) {
            Some(f) => f,
            None => {
                tracing::error!("[{}] File with id {} was not opened", client_addr, id);
                return;
            }
        };

        if let Err(e) = write_chunk_to_file(f, offset, buffer).await {
            tracing::error!(
                "[{}] Could not write chunk at offset 0x{:x} to {:?}: {}",
                client_addr,
                offset,
                id,
                e
            );
        } else {
            tracing::debug!(
                "[{}] Write {} bytes into {} at {}",
                client_addr,
                content_size,
                id,
                offset
            );
        }
        // });
    }

    async fn process_message_done(&mut self) {
        tracing::info!("[{}] Received done from client", self.client_addr());
    }

    pub async fn process_message(&mut self, message: Message) -> bool {
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
                id,
            } => self.process_message_file(filename, created, size, id).await,
            Message::FileChunk {
                id,
                offset,
                content_size,
                content,
            } => {
                self.process_message_file_chunk(id, offset, content_size, content)
                    .await
            }
            Message::Done => {
                self.process_message_done().await;
                return true;
            }
        }

        return false;
    }

    async fn process_buffer_internal(&mut self, buffer: &[u8]) -> Result<bool> {
        self.reassembler.push_data(buffer);

        match self.reassembler.get_next_data(&mut self.data) {
            Ok(()) => {
                let (rest, message) = Message::from_wire(&self.data[..])?;
                if !rest.is_empty() {
                    tracing::warn!("Got extra data at the end of the message");
                    tracing::warn!("Extra data: {:x?}", rest);
                }
                self.data.clear();
                let done = self.process_message(message).await;
                Ok(done)
            }
            Err(Error::NoData) => Ok(false),
            Err(Error::Deserialize(nom::Err::Incomplete(n))) => {
                match n {
                    nom::Needed::Unknown => tracing::trace!("Missing some bytes"),
                    nom::Needed::Size(s) => tracing::trace!("Missing {} bytes", s),
                }

                Ok(false)
            }
            Err(e) => Err(e),
        }
    }

    pub async fn process_buffer(&mut self, buffer: &[u8]) -> bool {
        match self.process_buffer_internal(buffer).await {
            Ok(done) => done,
            Err(e) => {
                tracing::error!("[{}] error: {}", self.client_addr(), e);
                true
            }
        }
    }
}
