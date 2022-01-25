use serde_derive::{Deserialize, Serialize};
use std::io::Write;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::config::Config;
use crate::error::Result;

/// Represent a serialized messsage with re-emit counts, offset
#[derive(Serialize, Deserialize)]
pub struct EnvelopeHeader {
    pub emission_count: u8,
    pub current_emission: u8,
    pub offset: u16,
    pub size: u16,
    pub total_size: u64,
}

impl EnvelopeHeader {
    pub fn size() -> usize {
        static ENVELOPE_HEADER_SIZE: AtomicUsize = AtomicUsize::new(0);

        let cached_size = ENVELOPE_HEADER_SIZE.load(Ordering::Relaxed);
        if cached_size == 0 {
            let eh = EnvelopeHeader {
                emission_count: 2,
                current_emission: 1,
                offset: 3,
                size: 4,
                total_size: 5,
            };
            let size = bincode::serialized_size(&eh).expect("Cannot guess size of EnvelopeHeader")
                as usize;
            ENVELOPE_HEADER_SIZE.store(size, Ordering::Relaxed);
            size
        } else {
            cached_size
        }
    }
}

/// Yields each chunk to send
pub struct EnvelopeIterator<'d> {
    /// The actual data being sent
    data: &'d [u8],

    /// A owned buffer to avoid realloctions and to allow fragmentation
    buffer: Vec<u8>,

    /// The envelope header
    header: EnvelopeHeader,

    /// Chunk size used (the value is just cached to avoid getting it several times)
    chunk_size: usize,
}

impl<'d> EnvelopeIterator<'d> {
    pub fn new(data: &'d [u8], config: &Config) -> Result<Self> {
        let envelope_header_size = EnvelopeHeader::size();
        let chunk_size = if data.len() + envelope_header_size > config.mtu {
            config.mtu - envelope_header_size
        } else {
            data.len()
        };
        let buffer = Vec::with_capacity(config.mtu);

        let header = EnvelopeHeader {
            emission_count: config.remission_count.try_into()?,
            current_emission: 1,
            offset: 0,
            size: 0,
            total_size: data.len() as u64,
        };

        Ok(Self {
            data,
            buffer,
            header,
            chunk_size,
        })
    }

    fn get_chunk(&self) -> &'d [u8] {
        let offset = self.header.offset as usize;
        let rest = &self.data[offset..];
        if rest.len() > self.chunk_size {
            &rest[..self.chunk_size]
        } else {
            rest
        }
    }

    pub(crate) fn get_next_envelope(&mut self) -> Option<&[u8]> {
        self.buffer.clear();
        if self.header.current_emission <= self.header.emission_count {
            let chunk = self.get_chunk();
            self.header.size = chunk.len().try_into().expect("Chunk size too big?!");
            bincode::serialize_into(&mut self.buffer, &self.header)
                .expect("Could not serialize header into buffer");
            self.header.current_emission += 1;
            self.buffer.write(chunk).unwrap();
            Some(&self.buffer[..])
        } else {
            let offset = self.header.offset as usize;
            if offset < self.data.len() {
                self.header.current_emission = 1;
                let chunk = self.get_chunk();
                self.header.size = chunk.len().try_into().expect("Chunk size too big?!");
                bincode::serialize_into(&mut self.buffer, &self.header)
                    .expect("Could not serialize header into buffer");
                let chunk_size: u16 = self.chunk_size.try_into().expect("Chunk size too big?!");
                self.header.offset += chunk_size;
                self.buffer.write(chunk).unwrap();
                Some(&self.buffer[..])
            } else {
                None
            }
        }
    }
}
