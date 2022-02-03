use std::collections::HashMap;
use std::io::SeekFrom;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

use crate::config::Config;
use crate::messages::Message;
use crate::retransmit::Reassembler;
use crate::udp::UdpReader;
use crate::{Error, Result, Wire};

use tokio::fs::File;
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::mpsc;

pub struct Server {
    socket: UdpReader,
    config: Arc<Config>,
    root: PathBuf,
    handlers: HashMap<SocketAddr, mpsc::Sender<Vec<u8>>>,
    kill_tx: mpsc::Sender<SocketAddr>,
    kill_rx: mpsc::Receiver<SocketAddr>,
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

        let (kill_tx, kill_rx) = mpsc::channel(config.channel_size);

        Self {
            socket,
            config: Arc::new(config),
            root,
            handlers: HashMap::new(),
            kill_tx,
            kill_rx,
        }
    }

    pub async fn recv_message(&mut self) -> Result<()> {
        let mut buffer = vec![0u8; self.config.mtu];
        let (size, client_addr) = self.socket.recv_from(&mut buffer[..]).await?;

        let sender = self.handlers.entry(client_addr).or_insert_with(|| {
            tracing::info!("Creating new handler for {}", &client_addr);
            let (sender, receiver) = mpsc::channel(self.config.channel_size);

            let kill_tx = self.kill_tx.clone();
            let mut handler = ClientHandler {
                keep_alive: None,
                root: self.root.clone(),
                client_addr: client_addr.clone(),
                reassembler: Reassembler::new(&self.config),
                data: Vec::new(),
                opened_files: HashMap::new(),
                receiver,
                kill_tx,
                config: Arc::clone(&self.config),
            };

            tokio::spawn(async move {
                while let Some(buf) = handler.receiver.recv().await {
                    let done = handler.process_buffer(&buf[..]).await;
                    if done {
                        break;
                    }
                }

                if let Err(e) = handler.kill_tx.send(handler.client_addr).await {
                    tracing::error!(
                        "[{}] Could not notify server of my end: {}",
                        handler.client_addr,
                        e
                    );
                } else {
                    tracing::info!("[{}] Handler done", handler.client_addr);
                }
            });

            sender
        });

        buffer.truncate(size);
        if let Err(e) = sender.send(buffer).await {
            tracing::warn!("Handler is gone for {}: {}", &client_addr, e);
            self.handlers.remove(&client_addr);
        }

        if let Ok(addr) = self.kill_rx.try_recv() {
            tracing::info!("Removing handler for {}", &addr);
            self.handlers.remove(&addr);
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
    receiver: mpsc::Receiver<Vec<u8>>,
    kill_tx: mpsc::Sender<SocketAddr>,
    reassembler: Reassembler,
    data: Vec<u8>,
    root: PathBuf,
    opened_files: HashMap<u64, (File, u64)>,
    config: Arc<Config>,
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
        let client_addr = self.client_addr().clone();
        let real_filename = self.root.join(&filename);
        if !real_filename.starts_with(&self.root) {
            tracing::warn!("File {} not in {}, ignoring", filename, self.root.display());
            return;
        }

        match crate::utils::fs::create_file(&real_filename, size).await {
            Ok(f) => {
                tracing::info!(
                    "[{}] Created file {} of {} bytes (id: 0x{:x})",
                    client_addr,
                    real_filename.display(),
                    size,
                    id
                );
                self.opened_files.insert(id, (f, 0));
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
    }

    async fn process_message_file_chunk(
        &mut self,
        id: u64,
        offset: u64,
        content_size: u16,
        content: Vec<u8>,
    ) {
        async fn write_chunk_to_file(
            file: &mut File,
            file_offset: &mut u64,
            offset: u64,
            content: &[u8],
        ) -> Result<()> {
            if content.iter().all(|x| *x == 0) {
                tracing::warn!("Got all zero chunk at {}", offset);
            }

            if *file_offset != offset {
                if *file_offset > offset {
                    tracing::warn!(
                        "Must have missed a chunk. Expected {}, got {} ({} bytes behind)",
                        *file_offset,
                        offset,
                        *file_offset - offset
                    );
                } else {
                    tracing::warn!(
                        "Must have missed a chunk. Expected {}, got {} ({} bytes ahead)",
                        *file_offset,
                        offset,
                        offset - *file_offset
                    );
                }
                *file_offset = file.seek(SeekFrom::Start(offset)).await?;
            }
            file.write_all(content).await?;
            *file_offset += content.len() as u64;
            // f.flush().await?;

            Ok(())
        }

        let client_addr = self.client_addr().clone();

        // If content_size is 0, then the file has been sent
        if content_size == 0 {
            tracing::info!("[{}] Done receiving 0x{:x}", self.client_addr, id);
            self.opened_files.remove(&id);
            return;
        }

        let (f, file_offset) = match self.opened_files.get_mut(&id) {
            Some(f) => f,
            None => {
                tracing::error!("[{}] File with id {} was not opened", client_addr, id);
                return;
            }
        };

        // Is content contiguous to out internal buffer?
        let buffer = &content[..content_size as usize];
        if let Err(e) = write_chunk_to_file(f, file_offset, offset, buffer).await {
            tracing::error!(
                "[{}] Could not write chunk at offset 0x{:x} to {:?}: {}",
                client_addr,
                offset,
                id,
                e
            );
        }
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
