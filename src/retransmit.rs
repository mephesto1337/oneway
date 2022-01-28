use std::io;
use std::mem::size_of;

use crate::udp::UdpWriter;
use crate::{Config, Error, Result, Wire};

use nom::bytes::complete::tag;
use nom::error::context;
use nom::number::complete::be_u16;

/// Magic value "1WAY"
const RETRANSMIT_MAGIC: &[u8; 4] = b"1WAY";

/// The actual Retransmit header being set as a prefix for each data send/received
#[derive(Debug, Clone)]
struct RetransmitHeader {
    /// Size of the chunk
    size: u16,
}

impl RetransmitHeader {
    const fn size() -> usize {
        let magic_size = RETRANSMIT_MAGIC.len();
        let size_size = size_of::<u16>();

        magic_size + size_size
    }
}

pub const fn max_payload_size(mtu: usize) -> usize {
    debug_assert!(mtu > RetransmitHeader::size());

    mtu - RetransmitHeader::size()
}

impl Wire for RetransmitHeader {
    fn from_wire(input: &[u8]) -> Result<(&[u8], Self)> {
        let (rest, _magic) = context("RetransmitHeader/MAGIC", tag(RETRANSMIT_MAGIC))(input)?;
        let (rest, size) = context("RetransmitHeader/size", be_u16)(rest)?;

        Ok((rest, Self { size }))
    }

    fn to_wire<W: io::Write>(&self, mut writer: W) -> Result<usize> {
        writer.write_all(&RETRANSMIT_MAGIC[..])?;
        writer.write_all(&self.size.to_be_bytes()[..])?;

        Ok(Self::size())
    }
}

/// A Generic wrapper to send data over an unrelyable wire
#[derive(Debug)]
pub struct Retransmit {
    /// Current emission (from 1 to `total_emissions`)
    current_emission: usize,

    /// Total number of transmissions for the same message
    total_emissions: usize,

    /// Inner buffer to yield chunks
    buffer: Vec<u8>,
}

impl Retransmit {
    /// Construct new `Retransmit` with specified configuration
    pub fn new(data: &[u8], remission_count: usize, mtu: usize) -> Result<Self> {
        let buffer_size = data.len() + RetransmitHeader::size();
        if buffer_size > mtu {
            return Err(Error::PayloadTooLarge(buffer_size));
        }
        let mut buffer = Vec::with_capacity(buffer_size);
        let header = RetransmitHeader {
            size: data.len().try_into()?,
        };
        header.to_wire(&mut buffer)?;
        buffer.extend_from_slice(data);
        assert_eq!(buffer.len(), buffer_size);

        Ok(Self {
            current_emission: 1,
            total_emissions: remission_count,
            buffer,
        })
    }

    /// Yeilds each chunk to send prefixed with a `RetransmitHeader`
    fn get_next_chunk<'s>(&'s mut self) -> Option<&'s [u8]> {
        // First advance current_emission
        if self.current_emission <= self.total_emissions {
            self.current_emission += 1;
            return Some(&self.buffer[..]);
        }

        // Nothing can be advanced, so we are done
        None
    }

    /// Reset counters of `RetransmitHeader`
    fn reset(&mut self) {
        self.current_emission = 1;
    }

    /// Sends current request with repetitions
    pub async fn send(&mut self, socket: &UdpWriter) -> Result<()> {
        self.reset();

        while let Some(chunk) = self.get_next_chunk() {
            tracing::debug!("Sending {} bytes chunk", chunk.len());
            socket.send(chunk).await?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct Reassembler {
    /// Buffer for data being passed to this struct
    buffer: Vec<u8>,

    /// Read offset into the buffer
    offset: usize,

    /// MTU configured
    mtu: usize,

    /// Previous chunk seen
    previous_chunk: Vec<u8>,
}

impl Reassembler {
    pub fn new(config: &Config) -> Self {
        Self {
            buffer: Vec::with_capacity(config.mtu * 2),
            offset: 0,
            mtu: config.mtu,
            previous_chunk: Vec::with_capacity(config.mtu),
        }
    }

    fn get_available_data(&self) -> &[u8] {
        &self.buffer[self.offset..]
    }

    fn get_available_data_len(&self) -> usize {
        self.get_available_data().len()
    }

    fn consume(&mut self, count: usize) {
        assert!(self.offset + count <= self.buffer.len());
        self.offset += count;

        if self.offset * 2 > self.buffer.capacity() {
            let mut new_buffer = Vec::with_capacity(self.mtu * 2);
            new_buffer.extend_from_slice(self.get_available_data());
            std::mem::swap(&mut new_buffer, &mut self.buffer);
            self.offset = 0;
        }
    }

    pub fn push_data(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        tracing::trace!(
            "Adding {} bytes to buffer: new_len={}",
            data.len(),
            self.buffer.len()
        );
    }

    /// Reassemble and returns next data
    pub fn get_next_data(&mut self, data: &mut Vec<u8>) -> Result<()> {
        data.clear();
        // We could re-parse the header each time, but it is so small and cheap that caching it
        // would not worth it
        while self.get_available_data_len() > RetransmitHeader::size() {
            let (rest, header) = RetransmitHeader::from_wire(self.get_available_data())?;
            let size = header.size as usize;
            if rest.len() < size {
                return Err(Error::NoData);
            } else {
                let chunk = &rest[..size];
                if &self.previous_chunk[..] == chunk {
                    // If we just yield this chunk, ignore it but still consume the chunk from our
                    // buffer
                    self.consume(RetransmitHeader::size() + size);

                    continue;
                } else {
                    // The chunk does not match the previous one
                    data.extend_from_slice(chunk);
                    self.previous_chunk.clear();
                    self.previous_chunk.extend_from_slice(&data[..]);
                    self.consume(RetransmitHeader::size() + size);

                    return Ok(());
                }
            }
        }

        Err(Error::NoData)
    }
}
