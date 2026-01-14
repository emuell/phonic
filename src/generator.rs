//! Generator trait for sources that can be driven by sequencers.

use std::sync::{mpsc::SyncSender, Arc};
use std::time::Duration;

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    parameter::{Parameter, ParameterValueUpdate},
    source::{unique_source_id, Source},
    utils::db_to_linear,
    Error, MixerId, NotePlaybackId, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
    SourceTime,
};

// -------------------------------------------------------------------------------------------------

pub mod empty;
pub mod sampler;
#[cfg(feature = "fundsp")]
pub mod fundsp;

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

    /// By default 8. Maximum number of simultaneous voices in the generator.
    pub voices: usize,

    /// By default `None`, which means play on the main mixer. When set to some specific id,
    /// the source will be played on the given mixer instead of the default one.
    pub target_mixer: Option<MixerId>,

    /// By default false. When true, measure the CPU load of the generator source.
    /// CPU load can then be accessed via the generator's playback handle.
    pub measure_cpu_load: bool,

    /// Wallclock time rate of playback pos events, emitted via PlaybackStatusEvent
    /// in the player. By default one second to avoid unnecessary overhead.
    /// Set to e.g. Duration::from_secf32(1.0/30.0) to trigger events 30 times per second.
    /// Set to None to disable reporting.
    pub playback_pos_emit_rate: Option<Duration>,
}

impl Default for GeneratorPlaybackOptions {
    fn default() -> Self {
        Self {
            volume: 1.0,
            panning: 0.0,
            voices: 8,
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

    pub fn voices(mut self, voices: usize) -> Self {
        self.voices = voices;
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

    pub fn playback_pos_emit_rate(mut self, duration: std::time::Duration) -> Self {
        self.playback_pos_emit_rate = Some(duration);
        self
    }
    pub fn playback_pos_emit_disabled(mut self) -> Self {
        self.playback_pos_emit_rate = None;
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
        if self.voices == 0 {
            return Err(Error::ParameterError(format!(
                "playback options voice count is '{}'",
                self.voices
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
    /// Update a single generator automation parameter.
    SetParameter {
        id: FourCC,
        value: Owned<ParameterValueUpdate>,
    },
    /// Update multiple generator automation parameters.
    SetParameters {
        values: Owned<Vec<(FourCC, ParameterValueUpdate)>>,
    },
}

// -------------------------------------------------------------------------------------------------

/// Messages to control playback of and within a [`Generator`].
pub enum GeneratorPlaybackMessage {
    /// For transient generators which got added via `Player::play_generator`, this
    /// marks the generator as stopped and removes it from the mixer as soon as all voices
    /// finished playing. For fixed generators which got added via `Player::add_generator`,
    /// this only stops all playing notes and keeps the generator running.
    Stop,
    /// Trigger a playback event. All playback events keep the generator running in the mixer.
    Trigger { event: GeneratorPlaybackEvent },
}

// -------------------------------------------------------------------------------------------------

/// A [`Source`] that is driven by note events.
///
/// Generators extend the *static* `Source` trait to support event-driven playback of e.g. musical 
/// instruments or sample players. They respond to note-on/note-off events, velocity, pitch changes,
/// and custom parameters, while also supporting standard volume and panning controls via the playback
/// message queue.
///
/// Generators can be used in two ways:
/// 1. **Played** via [`Player::play_generator`](crate::Player::play_generator):
///    The generator is treated as a transient source. It will be automatically removed from the
///    mixer when it is stopped via its handle or when [`Player::stop_all_sources`](crate::Player::stop_all_sources)
///    is called. This is useful for one-shot generators or temporary sound sources.
///
/// 2. **Added** via [`Player::add_generator`](crate::Player::add_generator):
///    The generator is treated as a permanent source. It will remain in the mixer even when
///    stopped via its handle (which only stops playing notes) or when `stop_all_sources` is called.
///    It must be explicitly removed via [`Player::remove_generator`](crate::Player::remove_generator).
///    This is useful for instruments that should persist throughout the application's lifetime.
///
/// Generator parameters work similarly to [`Effect`](crate::Effect) parameters: they provide
/// automation capabilities and can be queried via [`parameters()`](Self::parameters).
pub trait Generator: Source {
    /// Convert the Generator impl into a boxed `dyn Generator`.
    ///
    /// Avoids double boxing when a generator impl already is a `Box<dyn Generator>`.
    fn into_box(self) -> Box<dyn Generator>
    where
        Self: Sized,
    {
        Box::new(self)
    }

    /// Name of the generator for display debugging purposes.
    fn generator_name(&self) -> String;

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

    /// Returns true when this generator gets removed after it received a
    /// [Stop](GeneratorPlaybackMessage::Stop) event.
    fn is_transient(&self) -> bool;
    /// Maintained by the player: mark generator as transient or fixed (not transient).
    fn set_is_transient(&mut self, is_transient: bool);

    /// Optional parameter descriptors for the generator.
    ///
    /// When returning parameters here, implement `process_parameter_update` too.
    fn parameters(&self) -> Vec<&dyn Parameter> {
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

    /// Process multiple parameter updates in a batch in the audio thread.
    ///
    /// The default impl applies all parameter changes individually, but some generators
    /// may override this to apply multiple changes more efficiently.
    fn process_parameter_updates(
        &mut self,
        values: &[(FourCC, ParameterValueUpdate)],
    ) -> Result<(), Error> {
        for (id, value) in values {
            self.process_parameter_update(*id, value)?
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Allow adding/using boxed `dyn Generator`s as `Source` impls.
impl Source for Box<dyn Generator> {
    fn sample_rate(&self) -> u32 {
        (**self).sample_rate()
    }

    fn channel_count(&self) -> usize {
        (**self).channel_count()
    }

    fn is_exhausted(&self) -> bool {
        (**self).is_exhausted()
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        (**self).write(output, time)
    }
}

/// Allow adding/using boxed `dyn Generator`s as `Generator` impls.
impl Generator for Box<dyn Generator> {
    fn into_box(self) -> Box<dyn Generator> {
        self
    }

    fn generator_name(&self) -> String {
        (**self).generator_name()
    }

    fn playback_id(&self) -> PlaybackId {
        (**self).playback_id()
    }

    fn playback_options(&self) -> &GeneratorPlaybackOptions {
        (**self).playback_options()
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        (**self).playback_message_queue()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        (**self).playback_status_sender()
    }

    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        (**self).set_playback_status_sender(sender)
    }

    fn is_transient(&self) -> bool {
        (**self).is_transient()
    }

    fn set_is_transient(&mut self, is_transient: bool) {
        (**self).set_is_transient(is_transient)
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        (**self).parameters()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        (**self).process_parameter_update(id, value)
    }
    fn process_parameter_updates(
        &mut self,
        values: &[(FourCC, ParameterValueUpdate)],
    ) -> Result<(), Error> {
        (**self).process_parameter_updates(values)
    }
}
