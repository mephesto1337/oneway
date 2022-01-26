use std::fmt;
use std::io;

type NomError<I> = nom::Err<nom::error::VerboseError<I>>;

/// Possible errors issued by this crate
#[derive(Debug)]
pub enum Error {
    /// Standart I/O error
    IO(io::Error),

    /// Deserialization error
    Deserialize(NomError<Vec<u8>>),

    /// Integer conversion
    IntegerConversion(std::num::TryFromIntError),

    /// Invalid time conversions
    Time(std::time::SystemTimeError),

    /// Configuration error,
    InvalidConfig { linenum: usize, line: String },

    /// ParseIntError
    ParseInt(std::num::ParseIntError),

    /// No chunk was received
    NoData,

    /// Missing parts
    MissingData(std::ops::Range<usize>),

    /// Invalid UTF-8
    UTF8(std::str::Utf8Error),
}

pub type Result<T> = ::std::result::Result<T, Error>;

impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::IO(e)
    }
}

impl<I> From<NomError<I>> for Error
where
    I: AsRef<[u8]>,
{
    fn from(e: NomError<I>) -> Self {
        Self::Deserialize(match e {
            nom::Err::Incomplete(i) => nom::Err::Incomplete(i),
            nom::Err::Error(mut e) => {
                let errors = e
                    .errors
                    .drain(..)
                    .map(|(input, code)| (input.as_ref().to_vec(), code))
                    .collect();
                nom::Err::Error(nom::error::VerboseError { errors })
            }
            nom::Err::Failure(mut e) => {
                let errors = e
                    .errors
                    .drain(..)
                    .map(|(input, code)| (input.as_ref().to_vec(), code))
                    .collect();
                nom::Err::Error(nom::error::VerboseError { errors })
            }
        })
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

impl From<std::num::ParseIntError> for Error {
    fn from(e: std::num::ParseIntError) -> Self {
        Self::ParseInt(e)
    }
}

impl From<std::str::Utf8Error> for Error {
    fn from(e: std::str::Utf8Error) -> Self {
        Self::UTF8(e)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IO(ref e) => fmt::Display::fmt(e, f),
            Self::Deserialize(ref e) => write!(f, "Deserialization error: {}", e),
            Self::IntegerConversion(ref e) => fmt::Display::fmt(e, f),
            Self::Time(ref e) => fmt::Display::fmt(e, f),
            Self::InvalidConfig { linenum, ref line } => {
                write!(f, "Invalid line ({}) found in config: {}", linenum, line)
            }
            Self::ParseInt(ref e) => fmt::Display::fmt(e, f),
            Self::NoData => write!(f, "No chunk was received"),
            Self::MissingData(ref r) => write!(f, "Missing data from {} to {}", r.start, r.end),
            Self::UTF8(ref e) => fmt::Display::fmt(e, f),
        }
    }
}
