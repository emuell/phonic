//! Generator trait for sources that can be driven by sequencers.

use std::sync::{mpsc::SyncSender, Arc};
use std::time::Duration;

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    parameter::{ClonableParameter, ParameterValueUpdate},
    source::{unique_source_id, Source},
    utils::db_to_linear,
    Error, MixerId, NotePlaybackId, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

pub mod sampler;

#[cfg(feature = "fundsp")]
pub mod fundsp;

pub mod r#dyn;

// -------------------------------------------------------------------------------------------------

/// Generates a unique source id for a triggered note in a generator.
pub(crate) fn unique_note_id() -> usize {
    // Note id's are used as source ids when tracking playback status...
    unique_source_id()
}

// -------------------------------------------------------------------------------------------------

/// Options for playing back a generator source.
#[derive(Debug, Clone, Copy)]
pub struct GeneratorPlaybackOptions {
    /// By default 1.0f32. Customize to lower or raise the volume of the generator output.
    pub volume: f32,

    /// By default 0.0f32. Set in range -1.0..=1.0 to adjust generator's output panning position.
    pub panning: f32,

    /// By default None, which means play on the main mixer. When set to some specific id,
    /// the source will be played on the given mixer instead of the default one.
    pub target_mixer: Option<MixerId>,

    /// By default false. When true, measure the CPU load of the generator source.
    /// CPU load can then be accessed via the generator's playback handle.
    pub measure_cpu_load: bool,

    /// Wallclock time rate of playback pos events, emitted via PlaybackStatusEvent
    /// in the player. By default one second to avoid unnecessary overhead.
    /// Set to e.g. Duration::from_secf32(1.0/30.0) to trigger events 30 times per second.
    pub playback_pos_emit_rate: Option<Duration>,
}

impl Default for GeneratorPlaybackOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            panning: 0.0,
            target_mixer: None,
            measure_cpu_load: false,
            playback_pos_emit_rate: Some(Duration::from_secs(1)),
        }
    }
}

impl GeneratorPlaybackOptions {
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

    pub fn playback_pos_emit_rate(mut self, duration: std::time::Duration) -> Self {
        self.playback_pos_emit_rate = Some(duration);
        self
    }

    pub fn target_mixer(mut self, mixer_id: MixerId) -> Self {
        self.target_mixer = Some(mixer_id);
        self
    }

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

/// Events to start/stop, change playback properties or parameters **within** a [`Generator`].
pub enum GeneratorPlaybackEvent {
    /// Trigger a note on event.
    NoteOn {
        note_id: NotePlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        context: Option<PlaybackStatusContext>,
    },
    /// Trigger a note off event for a specific note playback.
    NoteOff { note_id: NotePlaybackId },
    /// Stop all currently playing notes.
    AllNotesOff,

    /// Set the speed/pitch of a specific note playback.
    SetSpeed {
        note_id: NotePlaybackId,
        speed: f64,
        glide: Option<f32>,
    },
    /// Set the volume of a specific note playback.
    SetVolume {
        note_id: NotePlaybackId,
        volume: f32,
    },
    /// Set the panning of a specific note playback.
    SetPanning {
        note_id: NotePlaybackId,
        panning: f32,
    },

    /// Update a generator automation parameter.
    SetParameter {
        id: FourCC,
        value: Owned<ParameterValueUpdate>,
    },
}

// -------------------------------------------------------------------------------------------------

/// Messages to control playback of and within a [`Generator`].
pub enum GeneratorPlaybackMessage {
    /// Stop the generator and remove it from the mixer. This stops playback, waits until the
    /// source is exhausted and then finally removes the source from the mixer.
    Stop,
    /// Trigger a playback event. All playback events keep the generator running in the mixer.
    Trigger { event: GeneratorPlaybackEvent },
}

// -------------------------------------------------------------------------------------------------

/// A [`Source`] that is driven by note events.
///
/// It supports the usual volume and panning events and additional note trigger events via
/// its playback message queue.
///
/// A generator is active as long as it get's actively stopped. Stopping will remove the
/// generator from it's parent mixer, so to keep it running stop all playing notes only instead.
///
/// Generator parameters work similarly to [`Effect`](crate::Effect) parameters: they provide
/// automation capabilities and can be queried via [`parameters()`](Self::parameters).
pub trait Generator: Source {
    /// A unique ID, which can be used to identify sources in `PlaybackStatusEvent`s.
    fn playback_id(&self) -> PlaybackId;

    /// The generator's playback options
    fn playback_options(&self) -> &GeneratorPlaybackOptions;

    /// Get the playback message queue for this generator.
    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>>;

    /// Channel to receive playback status from the generator.
    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>>;
    /// Set the playback status sender for this generator.
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>);

    /// Optional parameter descriptors for the generator.
    ///
    /// When returning parameters here, implement `process_parameter_update` too.
    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![]
    }

    /// Process a parameter update for this generator in the audio thread.
    fn process_parameter_update(
        &mut self,
        _id: FourCC,
        _value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        debug_assert!(
            self.parameters().is_empty(),
            "When providing parameters, implement 'process_parameter_update' too!"
        );
        Ok(())
    }
}
