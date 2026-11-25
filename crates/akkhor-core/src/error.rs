use std::fmt;

use crate::Format;

pub type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    /// The bytes don't look like any format the engine knows.
    UnknownFormat,
    /// The format was recognised, but its support is switched off at build time.
    FormatDisabled(Format),
    /// The file is a known format but is damaged or uses something unsupported.
    Malformed(String),
    /// The file is DRM-protected.
    Protected,
    /// A page, chapter or entry index is out of range.
    OutOfRange,
    /// A caller-supplied buffer is too small for the requested render.
    BufferTooSmall {
        needed: usize,
        got: usize,
    },
}

impl Error {
    pub fn malformed(msg: impl Into<String>) -> Self {
        Error::Malformed(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "io error: {e}"),
            Error::UnknownFormat => write!(f, "unknown file format"),
            Error::FormatDisabled(fmt) => {
                write!(f, "{fmt:?} support is not compiled into this build")
            }
            Error::Malformed(m) => write!(f, "malformed file: {m}"),
            Error::Protected => write!(f, "file is DRM-protected"),
            Error::OutOfRange => write!(f, "index out of range"),
            Error::BufferTooSmall { needed, got } => {
                write!(f, "buffer too small: need {needed} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e)
    }
}
