use std::{
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;

use crate::{
    source::{
        status::{PlaybackStatusContext, PlaybackStatusEvent},
        Source,
    },
    utils::db_to_linear,
    Error, MixerId, PlaybackId,
};

// -------------------------------------------------------------------------------------------------

pub mod common;

#[cfg(feature = "fundsp")]
pub mod fundsp;

// -------------------------------------------------------------------------------------------------

/// Options to control playback properties of a [`SynthSource`].
#[derive(Clone, Copy)]
pub struct SynthPlaybackOptions {
    /// By default 1.0f32. Customize to lower or raise the volume of the synth tone.
    pub volume: f32,

    /// By default 0.0f32. Set in range -1.0..=1.0 to adjust panning position.
    pub panning: f32,

    /// By default None: when set, the synth tone should start playing at the given
    /// sample frame time in the audio output stream.
    pub start_time: Option<u64>,

    /// By default None: when set, the source's volume will fade in with the given
    /// amount when starting to play.
    pub fade_in_duration: Option<Duration>,
    /// By default 5ms: volume fade-out duration, applied when the the source gets
    /// stopped before it finished playing.
    pub fade_out_duration: Option<Duration>,

    /// Wallclock time rate of playback pos events, emitted via PlaybackStatusEvent
    /// in the player. By default one second to avoid unnecessary overhead.
    /// Set to e.g. Duration::from_secf32(1.0/30.0) to trigger events 30 times per second.
    pub playback_pos_emit_rate: Option<Duration>,

    /// By default None, which means play on the main mixer. When set to some specific id,
    /// the source will be played on the given mixer instead of the default one.
    pub target_mixer: Option<MixerId>,

    /// By default false. When true, measure the CPU load of the synth source.
    /// CPU load can then be accessed via the source's playback handle.
    pub measure_cpu_load: bool,
}

impl Default for SynthPlaybackOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            panning: 0.0,
            start_time: None,
            fade_in_duration: None,
            fade_out_duration: Some(Duration::from_millis(50)),
            playback_pos_emit_rate: Some(Duration::from_secs(1)),
            target_mixer: None,
            measure_cpu_load: false,
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

    pub fn panning(mut self, panning: f32) -> Self {
        self.panning = panning;
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

    pub fn target_mixer(mut self, mixer_id: MixerId) -> Self {
        self.target_mixer = Some(mixer_id);
        self
    }

    /// Set whether to measure the CPU load of this source.
    pub fn measure_cpu_load(mut self, measure: bool) -> Self {
        self.measure_cpu_load = measure;
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
        if !(-1.0..=1.0).contains(&self.panning) || self.panning.is_nan() {
            return Err(Error::ParameterError(format!(
                "playback options 'panning' value is '{}'",
                self.panning
            )));
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Events to control playback of a [`SynthSource`]
pub enum SynthPlaybackMessage {
    /// Stop the synth source
    Stop,
}

// -------------------------------------------------------------------------------------------------

/// A [`Source`] which creates samples from a synthesized signal.
pub trait SynthSource: Source {
    /// Name of the synth for debugging or display purposes.
    fn synth_name(&self) -> String;

    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s
    fn playback_id(&self) -> PlaybackId;

    /// The synth source's playback options
    fn playback_options(&self) -> &SynthPlaybackOptions;

    /// Message queue to control this sources's playback.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<SynthPlaybackMessage>>;

    /// Channel to receive file playback status from the synth.
    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>>;
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>);

    /// Optional context passed along with the playback status.
    fn playback_status_context(&self) -> Option<PlaybackStatusContext>;
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>);
}
