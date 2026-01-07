//! FunDSP-based polyphonic generator.

use std::{
    collections::HashMap,
    sync::{mpsc::SyncSender, Arc},
};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;
use fundsp::{audiounit::AudioUnit, shared::Shared};

use crate::{
    generator::{GeneratorPlaybackEvent, GeneratorPlaybackMessage},
    parameter::{Parameter, ParameterValueUpdate},
    source::{unique_source_id, Source, SourceTime},
    utils::buffer::clear_buffer,
    Error, Generator, GeneratorPlaybackOptions, NotePlaybackId, PlaybackId, PlaybackStatusContext,
    PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

mod ahdsr;
mod parameter;
mod voice;

pub use ahdsr::{shared_ahdsr, SharedAhdsrNode};

use parameter::SharedParameterValue;
use voice::FunDspVoice;

// -------------------------------------------------------------------------------------------------

/// A polyphonic generator using FunDSP audio units created from a factory function.
///
/// The factory function receives shared variables for `gate`, `frequency`, `volume`, `panning`
/// and optional user defined shared parameters to control playback, and returns a FunDSP audio
/// unit that uses these as variables.
///
/// # Example
/// ```rust
/// use phonic::{GeneratorPlaybackOptions, generators::FunDspGenerator};
/// use phonic::fundsp::hacker32::*;
///
/// // Simple fundsp generator without additional parameters
/// let generator = FunDspGenerator::new(
///     "example_synth",
///     |gate: Shared, freq: Shared, vol: Shared, pan: Shared| {
///         // Simple saw wave with envelope and panning
///         let envelope = var(&gate) >> follow(0.01);
///         let sound = var(&freq) >> saw();
///         Box::new((envelope * sound * var(&vol) | var(&pan)) >> panner())
///     },
///     GeneratorPlaybackOptions::default(), // default playback options
///     44100,  // voice and source's sample rate
/// );
/// ```
pub struct FunDspGenerator {
    synth_name: Arc<String>,
    playback_id: PlaybackId,
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    voices: Vec<FunDspVoice>,
    active_voices: usize,
    shared_parameters: HashMap<FourCC, SharedParameterValue>,
    transient: bool, // True if the generator can exhaust
    stopping: bool,  // True if stop has been called and we are waiting for voices to decay
    stopped: bool,   // True if all voices have decayed after a stop call
    options: GeneratorPlaybackOptions,
    output_sample_rate: u32,
    output_channel_count: usize,
}

impl FunDspGenerator {
    /// Create a new FunDSP-based generator with the given voice count.
    ///
    /// # Arguments
    /// * `synth_name` - A name for the synth (for playback status tracking and debugging).
    /// * `voice_factory` - Function that creates a voice unit with given
    ///   (frequency, volume, gate, panning) shared variables.
    /// * `options` - Generic generator playback options.
    /// * `sample_rate` - Output sample rate.
    pub fn new<S: AsRef<str>, F>(
        synth_name: S,
        voice_factory: F,
        options: GeneratorPlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error>
    where
        F: Fn(Shared, Shared, Shared, Shared) -> Box<dyn AudioUnit>,
    {
        let synth_name = Arc::new(synth_name.as_ref().to_owned());

        let playback_id = unique_source_id();
        let playback_status_send = None;

        // Create playback message queue with space for trigger events only
        let playback_message_queue_size: usize = 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(playback_message_queue_size));

        // Pre-allocate all voices
        let mut voices = Vec::with_capacity(options.voices);
        let mut output_channel_count = 0;
        for _ in 0..options.voices {
            // Create common shared variables for this voice
            let gate = Shared::new(0.0);
            let frequency = Shared::new(440.0);
            let volume = Shared::new(1.0);
            let panning = Shared::new(0.0);

            // Create the voice node using the factory
            let mut audio_unit = voice_factory(
                gate.clone(),
                frequency.clone(),
                volume.clone(),
                panning.clone(),
            );
            audio_unit.set_sample_rate(output_sample_rate as f64);
            audio_unit.allocate();

            // Memorize voice channel count
            assert!(
                output_channel_count == 0 || output_channel_count == audio_unit.outputs(),
                "Channel layout should be the same for every created voice"
            );
            output_channel_count = audio_unit.outputs();

            // Create voice
            voices.push(FunDspVoice::new(
                Arc::clone(&synth_name),
                audio_unit,
                frequency,
                volume,
                panning,
                gate,
                options.playback_pos_emit_rate,
                output_sample_rate,
            ));
        }
        let active_voices = 0;

        let shared_parameters = HashMap::new();

        let transient = false;
        let stopping = false;
        let stopped = false;

        Ok(Self {
            synth_name,
            playback_id,
            playback_message_queue,
            playback_status_send,
            voices,
            active_voices,
            shared_parameters,
            transient,
            stopping,
            stopped,
            options,
            output_sample_rate,
            output_channel_count,
        })
    }

    /// Create a new FunDSP-based generator with the given voice count and shared parameters.
    ///
    /// # Arguments
    /// * `synth_name` - A name for the synth (for playback status tracking and debugging).
    /// * `parameters` - A slice of parameters which will be passed to the factory
    ///   in order automate vars within the factory.
    /// * `parameter_state` - Optional parameter values that should be applied initially.
    ///   When None, the parameters will be initialized with their default values.
    /// * `voice_factory` - Function that creates a voice unit with given
    ///   (frequency, volume, gate, panning) shared variables.
    /// * `options` - Generic generator playback options.
    /// * `sample_rate` - Output sample rate.
    pub fn with_parameters<S: AsRef<str>, F>(
        synth_name: S,
        parameters: &[&dyn Parameter],
        parameter_state: Option<&[(FourCC, ParameterValueUpdate)]>,
        voice_factory: F,
        options: GeneratorPlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error>
    where
        F: Fn(
            Shared,
            Shared,
            Shared,
            Shared,
            &mut dyn FnMut(FourCC) -> Shared,
        ) -> Box<dyn AudioUnit>,
    {
        let synth_name = Arc::new(synth_name.as_ref().to_owned());

        let playback_id = unique_source_id();
        let playback_status_send = None;

        // Create playback message queue to hold automation for all params at once + a bit more
        let playback_message_queue_size: usize = parameters.len() * 2 + 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(playback_message_queue_size));

        // Create parameter map and ensure that all parameter IDs are unique
        let mut shared_parameters = HashMap::with_capacity(parameters.len());
        for p in parameters {
            if shared_parameters
                .insert(p.id(), SharedParameterValue::from_description(*p))
                .is_some()
            {
                return Err(Error::ParameterError(format!(
                    "Duplicate parameter ID '{}' in parameter set",
                    p.id()
                )));
            }
        }

        // Apply initial parameter state, if any
        if let Some(parameter_state) = parameter_state {
            for (id, value) in parameter_state {
                if let Some(shared_parameter) = shared_parameters.get_mut(id) {
                    shared_parameter.apply_update(value);
                } else {
                    return Err(Error::ParameterError(format!(
                        "invalid parameter ID '{id}' in initial parameter state set"
                    )));
                }
            }
        }

        // Allocate all voices
        let mut voices = Vec::with_capacity(options.voices);
        let mut output_channel_count = 0;
        for _ in 0..options.voices {
            // Create common shared variables for this voice
            let gate = Shared::new(0.0);
            let frequency = Shared::new(440.0);
            let volume = Shared::new(1.0);
            let panning = Shared::new(0.0);

            // Create the voice node using the factory
            let mut audio_unit = voice_factory(
                gate.clone(),
                frequency.clone(),
                volume.clone(),
                panning.clone(),
                &mut |id: FourCC| -> Shared {
                    shared_parameters
                        .get(&id)
                        .unwrap_or_else(|| {
                            panic!("Parameter '{id}' not found in provided parameter set")
                        })
                        .shared()
                        .clone()
                },
            );
            audio_unit.set_sample_rate(output_sample_rate as f64);
            audio_unit.allocate();

            // Memorize voice channel count
            assert!(
                output_channel_count == 0 || output_channel_count == audio_unit.outputs(),
                "Channel layout should be the same for every created voice"
            );
            output_channel_count = audio_unit.outputs();

            // Create voice
            voices.push(FunDspVoice::new(
                Arc::clone(&synth_name),
                audio_unit,
                frequency,
                volume,
                panning,
                gate,
                options.playback_pos_emit_rate,
                output_sample_rate,
            ));
        }
        let active_voices = 0;

        let transient = false;
        let stopping = false;
        let stopped = false;

        Ok(Self {
            synth_name,
            playback_id,
            playback_message_queue,
            playback_status_send,
            voices,
            active_voices,
            shared_parameters,
            transient,
            stopping,
            stopped,
            options,
            output_sample_rate,
            output_channel_count,
        })
    }

    fn next_free_voice_index(&mut self, _current_sample_frame: u64) -> usize {
        // Try to find an inactive voice first
        if let Some(index) = self.voices.iter().position(|v| !v.is_active()) {
            return index;
        }
        // If all voices are active, find the best candidate to steal.
        // Prioritize:
        //   a) Longest releasing voice (earliest release_start_frame)
        //   b) Oldest active voice (smallest playback_id)
        let mut candidate_index = 0;
        let mut earliest_release_time: Option<u64> = None;
        let mut oldest_note_id: Option<NotePlaybackId> = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if voice.is_releasing() {
                // If this voice is releasing, check if it's the longest releasing one
                if let Some(release_time) = voice.release_start_frame() {
                    if earliest_release_time.is_none_or(|earliest| release_time < earliest) {
                        earliest_release_time = Some(release_time);
                        candidate_index = index;
                    }
                }
            } else if voice.note_id().is_some() {
                // If this voice is playing, check if it's the oldest active one
                // Only consider playing voices if no releasing voice has been found yet
                // (i.e., earliest_release_time is still None)
                if earliest_release_time.is_none() {
                    if let Some(current_playback_id) = voice.note_id() {
                        if oldest_note_id.is_none_or(|oldest| current_playback_id < oldest) {
                            oldest_note_id = Some(current_playback_id);
                            candidate_index = index;
                        }
                    }
                }
            }
        }
        candidate_index
    }

    fn stop(&mut self, current_sample_frame: u64) {
        // Mark source as about to stop when this is a transient generator
        self.stopping = self.transient;
        // Stop all active voices, if any
        self.trigger_all_notes_off(current_sample_frame);
    }

    fn trigger_note_on(
        &mut self,
        note_id: NotePlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        current_sample_frame: u64,
        context: Option<PlaybackStatusContext>,
    ) {
        let voice_index = self.next_free_voice_index(current_sample_frame);
        let voice = &mut self.voices[voice_index];
        voice.start(
            note_id,
            note,
            volume.unwrap_or(1.0),
            panning.unwrap_or(0.0),
            context,
        );
        // Ensure we're checking in the upcoming `write` if any voice needs processing.
        self.active_voices += 1;
    }

    fn trigger_note_off(&mut self, note_id: NotePlaybackId, current_sample_frame: u64) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.stop(current_sample_frame);
            // NB: do not modify `active_voices` here: it's updated in `write`.
        }
    }

    fn trigger_all_notes_off(&mut self, current_sample_frame: u64) {
        for voice in &mut self.voices {
            voice.stop(current_sample_frame);
            // NB: do not modify `active_voices` here: it's updated in `write`.
        }
    }

    fn trigger_set_speed(&mut self, note_id: NotePlaybackId, speed: f64, glide: Option<f32>) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_speed(speed, glide, self.output_sample_rate);
        }
    }

    fn trigger_set_volume(&mut self, note_id: NotePlaybackId, volume: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_volume(volume);
        }
    }

    fn trigger_set_panning(&mut self, note_id: NotePlaybackId, panning: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_panning(panning);
        }
    }

    fn process_playback_messages(&mut self, current_sample_frame: u64) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    self.stop(current_sample_frame);
                }
                GeneratorPlaybackMessage::Trigger { event } => {
                    // Ignore all events while stopping
                    if !self.stopping {
                        match event {
                            GeneratorPlaybackEvent::AllNotesOff => {
                                self.trigger_all_notes_off(current_sample_frame);
                            }
                            GeneratorPlaybackEvent::NoteOn {
                                note_id,
                                note,
                                volume,
                                panning,
                                context,
                            } => {
                                self.trigger_note_on(
                                    note_id,
                                    note,
                                    volume,
                                    panning,
                                    current_sample_frame,
                                    context,
                                );
                            }
                            GeneratorPlaybackEvent::NoteOff { note_id } => {
                                self.trigger_note_off(note_id, current_sample_frame);
                            }
                            GeneratorPlaybackEvent::SetSpeed {
                                note_id,
                                speed,
                                glide,
                            } => {
                                self.trigger_set_speed(note_id, speed, glide);
                            }
                            GeneratorPlaybackEvent::SetVolume { note_id, volume } => {
                                self.trigger_set_volume(note_id, volume);
                            }
                            GeneratorPlaybackEvent::SetPanning { note_id, panning } => {
                                self.trigger_set_panning(note_id, panning);
                            }
                            GeneratorPlaybackEvent::SetParameter { id, value } => {
                                if let Err(err) = self.process_parameter_update(id, &value) {
                                    log::warn!("Failed to process parameter '{id}' update: {err}");
                                }
                            }
                            GeneratorPlaybackEvent::SetParameters { values } => {
                                if let Err(err) = self.process_parameter_updates(&values) {
                                    log::warn!("Failed to process parameter updates: {err}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        if let Some(parameter) = self.shared_parameters.get_mut(&id) {
            parameter.apply_update(value);
            Ok(())
        } else {
            Err(Error::ParameterError(format!("Unknown parameter '{id}'")))
        }
    }
}

impl Source for FunDspGenerator {
    fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn channel_count(&self) -> usize {
        self.output_channel_count
    }

    fn is_exhausted(&self) -> bool {
        self.stopped
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // Process pending messages, if any
        self.process_playback_messages(time.pos_in_frames);

        // Return empty handed when exhausted or when there are no active voices
        if self.stopped || (self.active_voices == 0 && !self.stopping) {
            return 0;
        }

        // Prepare output for mixing
        clear_buffer(output);

        // Mix all active voices
        let mut active_voices = 0;
        for voice in &mut self.voices {
            if voice.is_active() {
                voice.process(output, time);
                if voice.is_active() {
                    // count voices that are still active after processed
                    active_voices += 1;
                }
            }
        }

        // Update `active_voices` based on the actual state
        self.active_voices = active_voices;

        // If the generator was stopping and all voices become inactive report as stopped.
        if self.stopping && active_voices == 0 {
            self.stopped = true;
            if let Some(sender) = &self.playback_status_send {
                if let Err(err) = sender.send(PlaybackStatusEvent::Stopped {
                    id: self.playback_id,
                    path: Arc::clone(&self.synth_name),
                    context: None,
                    exhausted: true,
                }) {
                    log::warn!("Failed to send fundsp generator playback status event: {err}");
                }
            }
        }

        // We've cleared the entire buffer so report the entire buffer's len
        output.len()
    }
}

impl Generator for FunDspGenerator {
    fn generator_name(&self) -> String {
        self.synth_name.to_string()
    }

    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_options(&self) -> &GeneratorPlaybackOptions {
        &self.options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        self.playback_message_queue.clone()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_send = sender.clone();
        for voice in &mut self.voices {
            voice.set_playback_status_sender(sender.clone());
        }
    }

    fn is_transient(&self) -> bool {
        self.transient
    }
    fn set_is_transient(&mut self, is_transient: bool) {
        self.transient = is_transient
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        self.shared_parameters
            .values()
            .map(|p| p.description())
            .collect()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        self.process_parameter_update(id, value)
    }
}
