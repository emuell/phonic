//! Generator trait for sources that can be driven by sequencers.

use std::sync::{mpsc::SyncSender, Arc};
use std::time::Duration;

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    modulation::{ModulationSource, ModulationTarget},
    parameter::{Parameter, ParameterValueUpdate},
    source::{unique_source_id, Source},
    utils::db_to_linear,
    Error, MixerId, NotePlaybackId, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
    SourceTime,
};

// -------------------------------------------------------------------------------------------------

pub mod empty;
#[cfg(feature = "fundsp")]
pub mod fundsp;
pub mod sampler;

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

    /// Set or update a modulation routing.
    SetModulation {
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    },
    /// Remove a modulation routing.
    ClearModulation { source: FourCC, target: FourCC },
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
/// ## Playback
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
/// ## Parameters
///
/// Generator parameters work similarly to [`Effect`](crate::Effect) parameters: they provide
/// automation capabilities and can be queried via [`parameters()`](Self::parameters).
///
/// To enable parameters in custom generators:
/// - Implement [`parameters()`](Self::parameters) to return parameter descriptors
/// - Implement [`process_parameter_update()`](Self::process_parameter_update) to handle parameter
///   changes in the audio thread
/// - Optionally override [`process_parameter_updates()`](Self::process_parameter_updates) for
///   more efficient batch processing
///
/// ## Modulation
///
/// Generators also can optionally provide a modulation system where a custom set of modulation sources
/// (LFOs, envelopes, velocity, keytracking) can be routed to modulatable target parameters with an
/// user-configurable depth.
///
/// To enable modulation in custom generators:
/// - Implement [`modulation_sources()`](Self::modulation_sources) to define available modulation sources
/// - Implement [`modulation_targets()`](Self::modulation_targets) to define parameters that can be modulated
/// - Implement [`set_modulation()`](Self::set_modulation) and [`clear_modulation()`](Self::clear_modulation)
///   to configure modulation routings
///
/// See [`ModulationSource`] and [`ModulationTarget`] for more details.
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

    /// Optional modulation sources for this generator. By default none.
    ///
    /// When returning sources here, implement the rest of the modulation interface as well!
    fn modulation_sources(&self) -> Vec<ModulationSource> {
        vec![]
    }

    /// Returns parameters that can receive modulation. By default none.
    ///
    /// When returning targets here, implement the rest of the modulation interface as well!
    fn modulation_targets(&self) -> Vec<ModulationTarget> {
        vec![]
    }

    /// Set or update a modulation routing.
    ///
    /// # Arguments
    /// * `source` - Modulation source ID (must be one in `Self::modulation_sources()``)
    /// * `target` - Target parameter ID (must be one in `Self::modulatable_parameters()`)
    /// * `amount` - Modulation amount (-1.0..=1.0)
    /// * `bipolar` - If true, transforms unipolar sources (0.0-1.0) to bipolar (-1.0..1.0)
    ///   centered at 0.5. Use for sources like keytracking when you want
    ///   middle values to be neutral (no modulation).
    ///
    /// Returns error if source or target is invalid.
    fn set_modulation(
        &mut self,
        _source: FourCC,
        _target: FourCC,
        _amount: f32,
        _bipolar: bool,
    ) -> Result<(), Error> {
        // Default: not supported
        Err(Error::ParameterError(
            "Modulation routing not supported by this generator".to_string(),
        ))
    }

    /// Remove a modulation routing.
    fn clear_modulation(&mut self, _source: FourCC, _target: FourCC) -> Result<(), Error> {
        // Default: not supported
        Err(Error::ParameterError(
            "Modulation routing not supported by this generator".to_string(),
        ))
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

    fn weight(&self) -> usize {
        (**self).weight()
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

    fn modulation_sources(&self) -> Vec<ModulationSource> {
        (**self).modulation_sources()
    }

    fn modulation_targets(&self) -> Vec<ModulationTarget> {
        (**self).modulation_targets()
    }

    fn set_modulation(
        &mut self,
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), Error> {
        (**self).set_modulation(source, target, amount, bipolar)
    }

    fn clear_modulation(&mut self, source: FourCC, target: FourCC) -> Result<(), Error> {
        (**self).clear_modulation(source, target)
    }
}
