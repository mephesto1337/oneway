use std::cmp;
use std::io;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::udp::UdpWriter;
use crate::{Config, Error, Result, Wire};

use nom::bytes::complete::tag;
use nom::combinator::verify;
use nom::error::context;
use nom::number::complete::{be_u16, be_u32};

/// Magic value "1WAY"
const RETRANSMIT_MAGIC: &[u8; 4] = b"1WAY";

/// The actual Retransmit header being set as a prefix for each data send/received
#[derive(Debug, Clone)]
struct RetransmitHeader {
    /// Offset in the current frame
    offset: u16,

    /// Size of the current chunk
    size: u16,

    /// Total size of the data being transmitted
    total_size: u16,

    /// Identifier of the request
    id: u32,
}

impl RetransmitHeader {
    const fn size() -> usize {
        let magic_size = RETRANSMIT_MAGIC.len();
        let offset_size = size_of::<u16>();
        let size_size = size_of::<u16>();
        let total_size_size = size_of::<u16>();
        let id_size = size_of::<u32>();

        magic_size + offset_size + size_size + total_size_size + id_size
    }

    fn get_next_id() -> u32 {
        static NEXT_ID: AtomicU32 = AtomicU32::new(0);
        // static NEXT_ID_INITIALIZED: AtomicBool = AtomicBool::new(false);

        // if !NEXT_ID_INITIALIZED.load(Ordering::Relaxed) {
        //     let random_id = unsafe { crate::utils::get_random::<u32>().assume_init() };
        //     NEXT_ID.store(random_id, Ordering::Relaxed);
        //     NEXT_ID_INITIALIZED.store(true, Ordering::Relaxed);
        // }

        NEXT_ID.fetch_add(1, Ordering::Relaxed)
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
        let (rest, id) = context("RetransmitHeader/id", be_u32)(rest)?;

        Ok((
            rest,
            Self {
                offset,
                size,
                total_size,
                id,
            },
        ))
    }

    fn to_wire<W: io::Write>(&self, mut writer: W) -> Result<usize> {
        writer.write_all(&RETRANSMIT_MAGIC[..])?;
        writer.write_all(&self.offset.to_be_bytes()[..])?;
        writer.write_all(&self.size.to_be_bytes()[..])?;
        writer.write_all(&self.total_size.to_be_bytes()[..])?;
        writer.write_all(&self.id.to_be_bytes()[..])?;

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
            id: RetransmitHeader::get_next_id(),
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
            tracing::trace!("Will send {:?}", &self.header);
            return Some(&self.buffer[..]);
        }

        let advance: u16 = (self.mtu - RetransmitHeader::size()).try_into().unwrap();
        // If we cannot advance current_emission, then increase offset
        if self.header.offset + advance < self.header.total_size {
            self.current_emission = 1;
            self.header.offset += advance;

            self.prepare_buffer();
            tracing::trace!("Will send {:?}", &self.header);
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

    /// Preload header
    current_header: Option<RetransmitHeader>,

    /// Expected next identifier
    expected_id: Option<u32>,
}

impl Reassembler {
    pub fn new(config: &Config) -> Self {
        Self {
            buffer: Vec::with_capacity(config.mtu * 2),
            offset: 0,
            current_header: None,
            expected_id: None,
        }
    }

    fn get_data(&self) -> &[u8] {
        &self.buffer[self.offset..]
    }

    fn consume(&mut self, count: usize) {
        assert!(self.offset + count <= self.buffer.len());
        self.offset += count;
        self.current_header = None;

        if self.buffer.len() > 4096 {
            tracing::trace!(
                "Buffer is too large ({} bytes), shrinking it",
                self.buffer.len()
            );
            let mut new_buffer = self.buffer.split_off(self.offset);
            std::mem::swap(&mut new_buffer, &mut self.buffer);
        }
    }

    pub fn push_data(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
        tracing::trace!("Adding {} bytes to buffer", data.len());
    }

    /// If `self.current_header`, parses content in self.buffer to load
    fn load_header(&mut self) -> Result<&RetransmitHeader> {
        if self.current_header.is_none() {
            if self.get_data().len() < RetransmitHeader::size() {
                return Err(Error::NoData);
            }
            let (_rest, header) = RetransmitHeader::from_wire(self.get_data())?;
            if self.expected_id.is_none() {
                self.expected_id = Some(header.id);
            }
            tracing::trace!("Consume new RetransmitHeader");
            self.consume(RetransmitHeader::size());
            self.current_header = Some(header);
        }

        self.current_header.as_ref().ok_or_else(|| Error::NoData)
    }

    fn get_restransmits(
        &mut self,
        data: &mut Vec<u8>,
        expected_offset: u16,
    ) -> Result<RetransmitHeader> {
        self.load_header()?;
        let mut expected_id = *self.expected_id.as_ref().unwrap();

        loop {
            let current_header = self.load_header()?.clone();
            tracing::debug!(
                "current_header = {:?}, expected_id={}, expected_offset={}",
                current_header,
                expected_id,
                expected_offset
            );

            match current_header.id.cmp(&expected_id) {
                cmp::Ordering::Less => {
                    tracing::trace!("Got previous session, skipping");
                    self.consume(current_header.size as usize);
                    continue;
                }
                cmp::Ordering::Greater => {
                    tracing::warn!(
                        "Got a future session?! expected_id={} / received={}",
                        expected_id,
                        current_header.id
                    );
                    expected_id = current_header.id;
                    self.expected_id = Some(expected_id);
                }
                cmp::Ordering::Equal => {}
            }

            match current_header.offset.cmp(&expected_offset) {
                cmp::Ordering::Less => {
                    tracing::trace!("Got already seen offset, skipping");
                    self.consume(current_header.size as usize);
                    continue;
                }
                cmp::Ordering::Greater => {
                    tracing::warn!("Missed chunk of data");
                    let start = expected_offset as usize;
                    let end = current_header.offset as usize;
                    return Err(Error::MissingData(start..end));
                }
                cmp::Ordering::Equal => {
                    break;
                }
            }
        }

        let header = self.current_header.take().unwrap();
        tracing::debug!("Consuming this header \\o/ ! {:?}", &header);
        let data_size = header.size as usize;
        let header_data = &self.get_data()[..data_size];
        data.extend_from_slice(header_data);
        tracing::trace!("Consuming buffer data: {:?} / {:x?}", &header, header_data);
        self.consume(data_size);

        Ok(header)
    }

    /// Reassemble and returns next data
    pub fn get_next_data(&mut self) -> Result<Vec<u8>> {
        let mut data = Vec::new();
        let mut expected_offset = 0;

        loop {
            eprintln!("[BEFORE] expected_offset = {}", expected_offset);
            let header = self.get_restransmits(&mut data, expected_offset)?;
            eprintln!("[AFTER1] expected_offset = {}", expected_offset);
            expected_offset += dbg!(header.size);
            eprintln!("[AFTER2] expected_offset = {}", expected_offset);
            if dbg!(expected_offset) == dbg!(header.total_size) {
                break;
            }
        }

        // We just want to increment the `RetransmitHeader::id`
        *self.expected_id.as_mut().unwrap() += 1;
        tracing::debug!("Got complete message");

        Ok(data)
    }
}
