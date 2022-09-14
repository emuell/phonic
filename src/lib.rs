mod error;
mod output;
mod player;
mod source;
mod utils;
mod waveform;

pub use error::Error;
pub mod convert {
    pub use super::utils::{db_to_linear, linear_to_db, pitch_from_note, speed_from_note};
}
pub use output::{AudioOutput, AudioSink, DefaultAudioOutput, DefaultAudioSink};
pub use player::AudioFilePlayer;
pub use source::*;
pub use waveform::{generate_waveform, WaveformPoint};
