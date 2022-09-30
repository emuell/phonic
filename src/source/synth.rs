#[cfg(feature = "dasp")]
pub mod dasp;

use crossbeam_channel::Sender;
use std::time::Duration;

use crate::{player::AudioFilePlaybackId, source::AudioSource, utils::db_to_linear, Error};

// -------------------------------------------------------------------------------------------------

/// Options to control playback of a SynthSource.
#[derive(Clone, Copy)]
pub struct SynthPlaybackOptions {
    /// By default 1.0f32. Customize to lower or raise the volume of the synth tone.
    pub volume: f32,
    /// By default None: when set, the synth tone should start playing at the given
    /// sample frame time in the audio output stream.
    pub start_time: Option<u64>,

    /// By default None: when set, the source's volume will fade in with the given
    /// amount when starting to play.
    pub fade_in_duration: Option<Duration>,
    /// By default 5ms: volume fade-out duration, applied when the the source gets
    /// stopped before it finished playing.
    pub fade_out_duration: Option<Duration>,

    /// Wallclock time rate of playback pos events, emited via AudioFilePlaybackStatusEvent
    /// in the player. By default one second to avoid unnecessary overhead.
    /// Set to e.g. Duration::from_secf32(1.0/30.0) to trigger events 30 times per second.
    pub playback_pos_emit_rate: Option<Duration>,
}

impl Default for SynthPlaybackOptions {
    fn default() -> Self {
        Self {
            volume: 1.0f32,
            start_time: None,
            fade_in_duration: None,
            fade_out_duration: Some(Duration::from_millis(50)),
            playback_pos_emit_rate: Some(Duration::from_secs(1)),
        }
    }
}

impl SynthPlaybackOptions {
    pub fn volume(mut self, volume: f32) -> Self {
        self.volume = volume;
        self
    }
    pub fn volume_db(mut self, volume_db: f32) -> Self {
        self.volume = db_to_linear(volume_db);
        self
    }

    pub fn fade_in(mut self, duration: Duration) -> Self {
        self.fade_in_duration = Some(duration);
        self
    }
    pub fn fade_out(mut self, duration: Duration) -> Self {
        self.fade_out_duration = Some(duration);
        self
    }

    pub fn start_at_time(mut self, sample_time: u64) -> Self {
        self.start_time = Some(sample_time);
        self
    }

    pub fn playback_pos_emit_rate(mut self, duration: std::time::Duration) -> Self {
        self.playback_pos_emit_rate = Some(duration);
        self
    }

    /// Validate all parameters. Returns Error::ParameterError on errors.
    pub fn validate(&self) -> Result<(), Error> {
        if self.volume < 0.0 || self.volume.is_nan() {
            return Err(Error::ParameterError(format!(
                "playback options 'volume' value is '{}'",
                self.volume
            )));
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a synth source
pub enum SynthPlaybackMessage {
    /// Stop the synth source
    Stop,
}

// -------------------------------------------------------------------------------------------------

/// A source which creates samples from a synthesized signal.
pub trait SynthSource: AudioSource {
    /// Channel sender to control this sources's playback
    fn playback_message_sender(&self) -> Sender<SynthPlaybackMessage>;
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s
    fn playback_id(&self) -> AudioFilePlaybackId;
}
