use std::{error, fmt, io};

// -------------------------------------------------------------------------------------------------

/// Provides an enumeration of all possible errors reported by phonic.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    MediaFileNotFound,
    MediaFileProbeError,
    MediaFileSeekError,
    AudioDecodingError(Box<dyn error::Error + Send + Sync>),
    OutputDeviceError(Box<dyn error::Error + Send + Sync>),
    ResamplingError(Box<dyn error::Error + Send + Sync>),
    IoError(io::Error),
    ParameterError(String),
    SendError,
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MediaFileNotFound => write!(f, "Audio file not found"),
            Self::MediaFileProbeError => write!(f, "Audio file failed to probe"),
            Self::MediaFileSeekError => write!(f, "Audio file failed to seek"),
            Self::AudioDecodingError(err)
            | Self::OutputDeviceError(err)
            | Self::ResamplingError(err) => err.fmt(f),
            Self::IoError(err) => err.fmt(f),
            Self::ParameterError(str) => write!(f, "Invalid parameter: {str}"),
            Self::SendError => write!(f, "Failed to send into a channel"),
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl<T> From<crossbeam_channel::SendError<T>> for Error {
    fn from(_: crossbeam_channel::SendError<T>) -> Self {
        Error::SendError
    }
}
