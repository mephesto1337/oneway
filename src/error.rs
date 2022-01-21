use std::fmt;
use std::io;

/// Possible errors issued by this crate
#[derive(Debug)]
pub enum Error {
    /// Standart I/O error
    IO(io::Error),
}

pub type Result<T> = ::std::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::IO(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IO(ref e) => fmt::Display::fmt(e, f),
        }
    }
}
