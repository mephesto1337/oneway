use std::cmp::Ordering;
use std::io;
use std::marker::Unpin;
use std::mem::size_of;
use std::time::Duration;

use crate::{Error, Result, Wire};

use nom::bytes::complete::tag;
use nom::combinator::verify;
use nom::error::context;
use nom::number::complete::be_u16;

use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Magic value "1WAY"
const RETRANSMIT_MAGIC: &[u8; 4] = b"1WAY";

/// The actual Retransmit header being set as a prefix for each data send/received
#[derive(Debug)]
struct RetransmitHeader {
    /// Offset in the current frame
    offset: u16,

    /// Size of the current chunk
    size: u16,

    /// Total size of the data being transmitted
    total_size: u16,
}

impl RetransmitHeader {
    const fn size() -> usize {
        let magic_size = RETRANSMIT_MAGIC.len();
        let offset_size = size_of::<u16>();
        let size_size = size_of::<u16>();
        let total_size_size = size_of::<u16>();

        magic_size + offset_size + size_size + total_size_size
    }
}

pub const fn max_payload_size(mtu: usize) -> usize {
    debug_assert!(mtu > RetransmitHeader::size());

    mtu - RetransmitHeader::size()
}

impl Wire for RetransmitHeader {
    fn from_wire(input: &[u8]) -> Result<(&[u8], Self)> {
        let (rest, _magic) = context("RetransmitHeader/MAGIC", tag(RETRANSMIT_MAGIC))(input)?;
        let (rest, offset) = context("RetransmitHeader/offset", be_u16)(rest)?;
        let (rest, size) = context("RetransmitHeader/size", be_u16)(rest)?;
        let (rest, total_size) = context(
            "RetransmitHeader/total_size",
            verify(be_u16, |&ts| ts >= offset + size),
        )(rest)?;

        Ok((
            rest,
            Self {
                offset,
                size,
                total_size,
            },
        ))
    }

    fn to_wire<W: io::Write>(&self, mut writer: W) -> Result<usize> {
        writer.write_all(&RETRANSMIT_MAGIC[..])?;
        writer.write_all(&self.offset.to_be_bytes()[..])?;
        writer.write_all(&self.size.to_be_bytes()[..])?;
        writer.write_all(&self.total_size.to_be_bytes()[..])?;

        Ok(Self::size())
    }
}

/// A Generic wrapper to send data over an unrelyable wire
#[derive(Debug)]
pub struct Retransmit<'d> {
    /// header to prefix each request
    header: RetransmitHeader,

    /// MTU used to divide data in chunks not exceeding `mtu` bytes with retransmit header
    mtu: usize,

    /// Current emission (from 1 to `total_emissions`)
    current_emission: usize,

    /// Total number of transmissions for the same message
    total_emissions: usize,

    /// The actual data being sent
    data: &'d [u8],

    /// Inner buffer to yield chunks
    buffer: Vec<u8>,
}

impl<'d> Retransmit<'d> {
    /// Construct new `Retransmit` with specified configuration
    pub fn new(data: &'d [u8], remission_count: usize, mtu: usize) -> Result<Self> {
        let total_size: u16 = data.len().try_into()?;
        // Just checks that MTU will not overflow u16
        let _mtu: u16 = mtu.try_into()?;
        let header = RetransmitHeader {
            offset: 0,
            size: 0,
            total_size,
        };
        let buffer = Vec::with_capacity(data.len() + RetransmitHeader::size());

        Ok(Self {
            header,
            mtu,
            current_emission: 1,
            total_emissions: remission_count,
            data,
            buffer,
        })
    }

    /// Get current chunk to issue
    fn get_chunk(&self) -> &'d [u8] {
        let offset = self.header.offset as usize;

        let remaining = &self.data[offset..];
        if remaining.len() + RetransmitHeader::size() > self.mtu {
            let size = self.mtu - RetransmitHeader::size();
            &remaining[..size]
        } else {
            remaining
        }
    }

    /// Prepare inner buffer for sending
    fn prepare_buffer(&mut self) {
        self.buffer.clear();
        let chunk = self.get_chunk();
        // unwrap is OK because we checked that mtu < u16::MAX
        self.header.size = chunk.len().try_into().unwrap();
        self.header
            .to_wire(&mut self.buffer)
            .expect("Write into a vector is always OK");
        self.buffer.extend_from_slice(chunk);
    }

    /// Yeilds each chunk to send prefixed with a `RetransmitHeader`
    fn get_next_chunk<'s>(&'s mut self) -> Option<&'s [u8]> {
        // First advance current_emission
        if self.current_emission <= self.total_emissions {
            self.prepare_buffer();
            self.current_emission += 1;
            log::trace!("Will send {:?}", &self.header);
            return Some(&self.buffer[..]);
        }

        // If we cannot advance current_emission, then increase offset
        if self.header.offset < self.header.total_size {
            self.current_emission = 1;
            let advance: u16 = (self.mtu - RetransmitHeader::size()).try_into().unwrap();
            self.header.offset += advance;

            self.prepare_buffer();
            log::trace!("Will send {:?}", &self.header);
            return Some(&self.buffer[..]);
        }

        // Nothing can be advanced, so we are done
        None
    }

    /// Reset counters of `RetransmitHeader`
    fn reset(&mut self) {
        self.current_emission = 1;
        self.header.offset = 0;
    }

    /// Sends current request with repetitions
    pub async fn send(&mut self, mut writer: impl AsyncWriteExt + Unpin) -> Result<()> {
        self.reset();

        while let Some(chunk) = self.get_next_chunk() {
            writer.write_all(chunk).await?;
        }
        Ok(())
    }

    /// Small helper to receive data with a timeout
    async fn recv_exact(
        reader: &mut (impl AsyncReadExt + Unpin),
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<()> {
        let sleep = tokio::time::sleep(timeout);
        tokio::pin!(sleep);

        tokio::select! {
            result = reader.read_exact(buffer) => {
                let _ = result?;
                Ok(())
            }
            _ = &mut sleep => {
                Err(Error::NoData)
            }
        }
    }

    /// Received exactly one `RetransmitHeader` and its chunks if data
    async fn recv_one_chunk(
        reader: &mut (impl AsyncReadExt + Unpin),
        timeout: Duration,
        chunk: &mut Vec<u8>,
    ) -> Result<RetransmitHeader> {
        let mut header_buf = [0u8; RetransmitHeader::size()];
        Self::recv_exact(reader, &mut header_buf[..], timeout.clone()).await?;
        let (_rest, header) = RetransmitHeader::from_wire(&header_buf[..])?;

        // Always reads the chunlk associated with the header
        let size = header.size as usize;
        chunk.resize(size, 0);
        Self::recv_exact(reader, &mut chunk[..], timeout).await?;

        Ok(header)
    }

    /// Helper function to receive all retransmisted same chunks
    async fn recv_retransmits(
        reader: &mut (impl AsyncReadExt + Unpin),
        timeout: Duration,
        expected_offset: u16,
        chunk: &mut Vec<u8>,
    ) -> Result<RetransmitHeader> {
        loop {
            let header = Self::recv_one_chunk(reader, timeout.clone(), chunk).await?;
            match header.offset.cmp(&expected_offset) {
                Ordering::Less => {
                    // Previous chunk, just ignore it
                    continue;
                }
                Ordering::Greater => {
                    // We missed all previous data
                    let start_missing = expected_offset as usize;
                    let end_missing = header.offset as usize;
                    return Err(Error::MissingData(start_missing..end_missing));
                }
                Ordering::Equal => {
                    // We got our chunk, let's break
                    return Ok(header);
                }
            }
        }
    }

    /// Receive a single buffer with retransmit logic being taken care of
    pub async fn recv(
        mut reader: impl AsyncReadExt + Unpin,
        timeout: Duration,
        mtu: usize,
    ) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut expected_offset = 0;
        let mut chunk = Vec::with_capacity(mtu);

        loop {
            let header =
                Self::recv_retransmits(&mut reader, timeout.clone(), expected_offset, &mut chunk)
                    .await?;
            if data.len() != expected_offset as usize {
                return Err(Error::MissingData(data.len()..expected_offset as usize));
            }
            assert_eq!(chunk.len(), header.size as usize);
            data.extend_from_slice(&chunk[..]);
            expected_offset += header.size;

            if expected_offset >= header.total_size {
                break;
            }
        }

        Ok(data)
    }
}
