pub mod connection;
// pub mod envelope;
mod config;
mod error;
pub mod messages;
pub mod retransmit;
pub mod tree;
pub mod udp;
mod utils;

pub use config::Config;
pub use error::{Error, Result};

/// Trait used to serialize/deserialize data to/from wire
pub trait Wire: Sized {
    /// Deserialization function
    fn from_wire(input: &[u8]) -> Result<(&[u8], Self)>;

    /// Serialization function
    fn to_wire<W>(&self, writer: W) -> Result<usize>
    where
        W: std::io::Write;
}
