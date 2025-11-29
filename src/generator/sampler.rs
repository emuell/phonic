use std::{
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    generator::{GeneratorPlaybackEvent, GeneratorPlaybackMessage},
    parameter::{ClonableParameter, FloatParameter, Parameter, ParameterValueUpdate},
    source::{
        file::preloaded::PreloadedFileSource, mixed::MixedSource, unique_source_id, Source,
        SourceTime,
    },
    utils::{
        ahdsr::AhdsrParameters,
        buffer::{add_buffers, clear_buffer},
    },
    Error, FilePlaybackOptions, Generator, NotePlaybackId, PlaybackId, PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

mod voice;
use voice::SamplerVoice;

// -------------------------------------------------------------------------------------------------

/// A basic sampler that plays a single audio file from RAM with an optional AHDSR envelope on
/// a limited set of voices.
///
/// All sampler parameters can be automated.
pub struct Sampler {
    playback_id: PlaybackId,
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    file_path: Arc<String>,
    voices: Vec<SamplerVoice>,
    active_voices: usize,
    envelope_parameters: Option<AhdsrParameters>,
    active_parameters: Vec<Box<dyn ClonableParameter + Send + Sync>>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    stopping: bool, // True if stop has been called and we are waiting for voices to decay
    stopped: bool,  // True if all voices have decayed after a stop call
    output_sample_rate: u32,
    output_channel_count: usize,
    temp_buffer: Vec<f32>,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
    const MIN_TIME_SEC: f32 = 0.0;
    const MAX_TIME_SEC: f32 = 10.0;

    pub const AMP_ATTACK: FloatParameter = FloatParameter::new(
        FourCC(*b"AATK"),
        "Attack",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.001,
    )
    .with_unit("s");
    pub const AMP_HOLD: FloatParameter = FloatParameter::new(
        FourCC(*b"AHLD"),
        "Hold",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.75,
    )
    .with_unit("s");
    pub const AMP_DECAY: FloatParameter = FloatParameter::new(
        FourCC(*b"ADCY"),
        "Decay",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.5,
    )
    .with_unit("s");
    pub const AMP_SUSTAIN: FloatParameter = FloatParameter::new(
        FourCC(*b"ASTN"), //
        "Sustain",
        0.0..=1.0,
        0.75,
    );
    pub const AMP_RELEASE: FloatParameter = FloatParameter::new(
        FourCC(*b"AREL"),
        "Release",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        1.0,
    )
    .with_unit("s");

    /// Create a new sampler with the given sample file, optional AHDSR envelope
    /// and the given fixed voice count.
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        envelope_parameters: Option<AhdsrParameters>,
        playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
        voice_count: usize,
        output_channel_count: usize,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        let playback_id = unique_source_id();

        // Pre-allocate playback message queue
        const PLAYBACK_MESSAGE_QUEUE_SIZE: usize = 10 + 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(PLAYBACK_MESSAGE_QUEUE_SIZE));

        // Load sample file
        let file_path = Arc::new(path.as_ref().to_string_lossy().to_string());
        let sample = PreloadedFileSource::from_file(
            path,
            playback_status_send.clone(),
            Default::default(),
            output_sample_rate,
        )?;

        // Set voice playback options
        let mut voice_playback_options = FilePlaybackOptions::default();
        if envelope_parameters.is_none() {
            // just de-click when there's no envelope
            voice_playback_options.fade_out_duration = Some(Duration::from_millis(50));
        } else {
            // use envelope only
            voice_playback_options.fade_out_duration = None;
        }

        // Allocate voices
        let mut voices = Vec::with_capacity(voice_count);
        for _ in 0..voice_count {
            let file_source = sample
                .clone(voice_playback_options, output_sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!("Failed to create sampler voice: {err}"))
                })?;
            voices.push(SamplerVoice::new(file_source, output_channel_count));
        }

        // Initialize envelope parameters, if any
        let mut envelope_parameters = envelope_parameters;
        if let Some(envelope_parameters) = &mut envelope_parameters {
            envelope_parameters
                .set_sample_rate(output_sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!(
                        "Failed to create envelope parameters for sampler: {err}"
                    ))
                })?;
        }
        let active_voices = 0;

        // Collect active parameters
        let mut active_parameters = Vec::<Box<dyn ClonableParameter + Send + Sync>>::new();
        if envelope_parameters.is_some() {
            active_parameters.extend(
                [
                    Self::AMP_ATTACK,
                    Self::AMP_HOLD,
                    Self::AMP_DECAY,
                    Self::AMP_SUSTAIN,
                    Self::AMP_RELEASE,
                ]
                .map(Box::from)
                .map(|p| p as Box<dyn ClonableParameter + Send + Sync>),
            );
        }

        // Initial playback state
        let stopping = false;
        let stopped = false;

        // Pre-allocate temp buffer for mixing, using mixer's max sample buffer size
        let temp_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

        Ok(Self {
            playback_id,
            playback_message_queue,
            playback_status_send,
            file_path,
            voices,
            active_voices,
            envelope_parameters,
            active_parameters,
            stopping,
            stopped,
            output_sample_rate,
            output_channel_count,
            temp_buffer,
        })
    }

    /// Process pending playback messages from the queue.
    fn process_playback_messages(&mut self, current_sample_frame: u64) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    self.stop(current_sample_frame);
                }
                GeneratorPlaybackMessage::Trigger { event } => {
                    // Ignore all trigger messages while we're stopping
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
                            } => {
                                self.trigger_note_on(note_id, note, volume, panning);
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
                                    log::warn!("Failed to update sampler parameter {id:?}: {err}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn stop(&mut self, current_sample_frame: u64) {
        // Mark source as about to stop
        self.stopping = true;
        // Stop all active voices, if any
        self.trigger_all_notes_off(current_sample_frame);
    }

    /// Immediately trigger a note on (used by event processor)
    fn trigger_note_on(
        &mut self,
        note_id: NotePlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    ) {
        // Allocate a new voice
        let voice_index = self.next_free_voice_index();
        let voice = &mut self.voices[voice_index];
        voice.start(
            note_id,
            note,
            volume.unwrap_or(1.0),
            panning.unwrap_or(0.0),
            &self.envelope_parameters,
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
            voice.stop(&self.envelope_parameters, current_sample_frame);
            // NB: do not modify `active_voices` here. it's updated in `write`
        }
    }

    fn trigger_all_notes_off(&mut self, current_sample_frame: u64) {
        for voice in &mut self.voices {
            voice.stop(&self.envelope_parameters, current_sample_frame);
            // NB: do not modify `active_voices` here. it's updated in `write`
        }
    }

    fn trigger_set_speed(&mut self, note_id: NotePlaybackId, speed: f64, glide: Option<f32>) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_speed(speed, glide);
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

    /// Find a free voice or steal the oldest one.
    /// Returns the index of the new voice, which is always valid.
    fn next_free_voice_index(&self) -> usize {
        // Try to find a completely free voice first
        if let Some(index) = self.voices.iter().position(|v| !v.is_active()) {
            return index;
        }
        // If all voices are active, find the best candidate to steal
        // Prioritize:
        //   a) Longest releasing voice (earliest release_start_sample_frame)
        //   b) Oldest active voice (by playback_id)
        let mut candidate_index = 0;
        let mut earliest_release_time: Option<u64> = None;
        let mut oldest_active_playback_id: Option<NotePlaybackId> = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if self.envelope_parameters.is_some() && voice.in_release_stage() {
                // This voice is in Release stage
                if let Some(release_time) = voice.release_start_frame() {
                    if earliest_release_time.is_none_or(|earliest| release_time < earliest) {
                        earliest_release_time = Some(release_time);
                        oldest_active_playback_id = None; // Reset active voices once we found a releasing voice
                        candidate_index = index;
                    }
                }
            } else if earliest_release_time.is_none() {
                // This voice is active (not in Release stage)
                // Only consider if we haven't found a releasing voice yet
                if let Some(playback_id) = voice.note_id() {
                    if oldest_active_playback_id.is_none_or(|oldest| playback_id < oldest) {
                        oldest_active_playback_id = Some(playback_id);
                        candidate_index = index;
                    }
                }
            }
        }
        candidate_index
    }

    fn parameter_update_value(
        value: &ParameterValueUpdate,
        descriptor: &FloatParameter,
    ) -> Result<f32, Error> {
        match value {
            ParameterValueUpdate::Normalized(norm) => {
                Ok(descriptor.denormalize_value(norm.clamp(0.0, 1.0)))
            }
            ParameterValueUpdate::Raw(raw) => {
                if let Some(v) = raw.downcast_ref::<f32>() {
                    Ok(descriptor.clamp_value(*v))
                } else if let Some(v) = raw.downcast_ref::<f64>() {
                    Ok(descriptor.clamp_value(*v as f32))
                } else {
                    Err(Error::ParameterError(format!(
                        "Unsupported payload type for sampler parameter '{}'",
                        descriptor.name()
                    )))
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

impl Source for Sampler {
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
        // Process playback messages
        self.process_playback_messages(time.pos_in_frames);

        // Return empty handed when exhausted or when there are no active voices
        if self.stopped || (self.active_voices == 0 && !self.stopping) {
            return 0;
        }

        // Clear output
        clear_buffer(output);

        // Mix active voices into the output
        let mut active_voices = 0;
        for voice in &mut self.voices {
            if voice.is_active() {
                assert!(self.temp_buffer.len() >= output.len());
                let mix_buffer = &mut self.temp_buffer[..output.len()];
                clear_buffer(mix_buffer);
                let written = voice.process(
                    mix_buffer,
                    self.output_channel_count,
                    &self.envelope_parameters,
                    time,
                );
                add_buffers(&mut output[..written], &mix_buffer[..written]);
                if voice.is_active() {
                    // count voices that are still active after processed
                    active_voices += 1;
                }
            }
        }

        // Update `active_voices` based on the actual state
        self.active_voices = active_voices;

        // Send a stop message when we got requested to stop and are now exhausted
        if self.stopping && active_voices == 0 {
            self.stopped = true;
            if let Some(sender) = &self.playback_status_send {
                if let Err(err) = sender.send(PlaybackStatusEvent::Stopped {
                    id: self.playback_id,
                    path: self.file_path.clone(),
                    context: None,
                    exhausted: true,
                }) {
                    log::warn!("Failed to send sampler playback status event: {err}");
                }
            }
        }

        // We've cleared the entire buffer, so return the entire buffer
        output.len()
    }
}

impl Generator for Sampler {
    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        self.playback_message_queue.clone()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_send = sender;
    }

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        self.active_parameters
            .iter()
            .map(|p| p.as_ref() as &dyn ClonableParameter)
            .collect()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        let params = self.envelope_parameters.as_mut().ok_or_else(|| {
            Error::ParameterError("Sampler has no AHDSR envelope configured".to_string())
        })?;
        match id {
            _ if id == Self::AMP_ATTACK.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_ATTACK)?;
                params.set_attack_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_HOLD.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_HOLD)?;
                params.set_hold_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_DECAY.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_DECAY)?;
                params.set_decay_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_SUSTAIN.id() => {
                let sustain = Sampler::parameter_update_value(value, &Self::AMP_SUSTAIN)?;
                params.set_sustain_level(sustain)?;
            }
            _ if id == Self::AMP_RELEASE.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_RELEASE)?;
                params.set_release_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown sampler parameter: {id:?}"
                )))
            }
        }

        Ok(())
    }
}
