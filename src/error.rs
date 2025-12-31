use std::{error, fmt, io};

// -------------------------------------------------------------------------------------------------

/// Provides an enumeration of all possible errors reported by phonic.
#[derive(Debug)]
#[allow(clippy::enum_variant_names)]
pub enum Error {
    SourceNotPlaying,
    MediaFileNotFound,
    MediaFileProbeError,
    MediaFileSeekError,
    AudioDecodingError(Box<dyn error::Error + Send + Sync>),
    OutputDeviceError(Box<dyn error::Error + Send + Sync>),
    ResamplingError(Box<dyn error::Error + Send + Sync>),
    GeneratorNotFoundError(usize),
    EffectNotFoundError(usize),
    MixerNotFoundError(usize),
    ParameterError(String),
    SendError(String),
    IoError(io::Error),
}

impl error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceNotPlaying => write!(f, "Source is no longer playing"),
            Self::MediaFileNotFound => write!(f, "Audio file not found"),
            Self::MediaFileProbeError => write!(f, "Audio file failed to probe"),
            Self::MediaFileSeekError => write!(f, "Audio file failed to seek"),
            Self::AudioDecodingError(err)
            | Self::OutputDeviceError(err)
            | Self::ResamplingError(err) => err.fmt(f),
            Self::GeneratorNotFoundError(playback_id) => {
                write!(f, "Generator with id {playback_id} not found")
            }
            Self::MixerNotFoundError(mixer_id) => write!(f, "Mixer with id {mixer_id} not found"),
            Self::EffectNotFoundError(effect_id) => {
                write!(f, "Effect with id {effect_id} not found")
            }
            Self::ParameterError(str) => write!(f, "Invalid parameter: {str}"),
            Self::SendError(str) => write!(f, "Failed to send channel message: {str}"),
            Self::IoError(err) => err.fmt(f),
        }
    }
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Error {
        Error::IoError(err)
    }
}

impl<T> From<std::sync::mpsc::SendError<T>> for Error {
    fn from(err: std::sync::mpsc::SendError<T>) -> Self {
        Error::SendError(err.to_string())
    }
}

impl<T> From<std::sync::mpsc::TrySendError<T>> for Error {
    fn from(err: std::sync::mpsc::TrySendError<T>) -> Self {
        Error::SendError(err.to_string())
    }
}
