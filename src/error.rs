use std::fmt;
use std::io;

/// Possible errors issued by this crate
#[derive(Debug)]
pub enum Error {
    /// Standart I/O error
    IO(io::Error),

    /// Serialization error
    Serialize(bincode::Error),

    /// Deserialization error
    Deserialize(bincode::Error),

    /// Integer conversion
    IntegerConversion(std::num::TryFromIntError),

    /// Invalid time conversions
    Time(std::time::SystemTimeError),
}

pub type Result<T> = ::std::result::Result<T, Error>;

impl Error {
    pub fn to_serialization(self) -> Self {
        let new = match self {
            Self::Serialize(inner) => Some(inner),
            Self::Deserialize(inner) => Some(inner),
            _ => None,
        };
        Self::Serialize(new.expect("Error was not a Serialize/Deserialize one"))
    }

    pub fn to_deserialization(self) -> Self {
        let new = match self {
            Self::Serialize(inner) => Some(inner),
            Self::Deserialize(inner) => Some(inner),
            _ => None,
        };
        Self::Deserialize(new.expect("Error was not a Serialize/Deserialize one"))
    }
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::IO(e)
    }
}

impl From<bincode::Error> for Error {
    fn from(e: bincode::Error) -> Self {
        Self::Serialize(e)
    }
}

impl From<std::num::TryFromIntError> for Error {
    fn from(e: std::num::TryFromIntError) -> Self {
        Self::IntegerConversion(e)
    }
}

impl From<std::time::SystemTimeError> for Error {
    fn from(e: std::time::SystemTimeError) -> Self {
        Self::Time(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IO(ref e) => fmt::Display::fmt(e, f),
            Self::Serialize(ref e) => write!(f, "Serialization error: {}", e),
            Self::Deserialize(ref e) => write!(f, "Deserialization error: {}", e),
            Self::IntegerConversion(ref e) => fmt::Display::fmt(e, f),
            Self::Time(ref e) => fmt::Display::fmt(e, f),
        }
    }
}
