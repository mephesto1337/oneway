use serde_derive::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::Result;

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
        content: Vec<u8>,
        created: u64,
    },
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    emission_count: u8,
    current_emission: u8,
    message: Vec<u8>,
}

/// Yields each chunk to send
pub(crate) struct EnvelopeIterator<'c> {
    /// The inner envelope message beeing sent
    envelope: Envelope,

    /// A owned buffer to avoid realloctions and to allow fragmentation
    buffer: Vec<u8>,

    /// Next offset to send, if none we must rebuild the buffer first
    offset: Option<usize>,

    /// A reference to the configuration beeing used
    config: &'c Config,
}

impl<'c> EnvelopeIterator<'c> {
    fn new(message: &Message, config: &'c Config) -> Result<Self> {
        let raw_message = bincode::serialize(message)?;
        let emission_count: u8 = config.remission_count.try_into()?;

        let envelope = Envelope {
            emission_count,
            current_emission: 0,
            message: raw_message,
        };
        let serialized_size: usize = bincode::serialized_size(&envelope)?.try_into()?;
        let buffer = Vec::with_capacity(serialized_size);
        let offset = None;

        Ok(Self {
            envelope,
            buffer,
            offset,
            config,
        })
    }

    pub(crate) fn get_next_envelope(&mut self) -> Option<&[u8]> {
        if let Some(offset) = self.offset.take() {
            let mtu = self.config.mtu;
            let chunk = &self.buffer[offset..][..mtu];
            if offset + mtu > self.buffer.len() {
                self.offset = None;
            } else {
                self.offset = Some(offset + mtu);
            }
            Some(chunk)
        } else {
            // We must rebuild our buffer
            if self.envelope.current_emission < self.envelope.emission_count {
                bincode::serialize_into(&mut self.buffer, &self.envelope)
                    .expect("Could not serialize an Envelope");
                self.envelope.current_emission += 1;
                self.offset = Some(0);
                let chunk = &self.buffer[..self.config.mtu];
                Some(chunk)
            } else {
                None
            }
        }
    }
}

impl Message {
    pub(crate) fn get_envelopes<'c>(&self, config: &'c Config) -> Result<EnvelopeIterator<'c>> {
        Ok(EnvelopeIterator::new(self, config)?)
    }
}
