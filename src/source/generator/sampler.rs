use std::{
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    parameter::{ClonableParameter, FloatParameter, Parameter, ParameterValueUpdate},
    source::{
        amplified::AmplifiedSource,
        file::preloaded::PreloadedFileSource,
        generator::{GeneratorPlaybackEvent, GeneratorPlaybackMessage},
        mapped::ChannelMappedSource,
        mixed::MixedSource,
        panned::PannedSource,
        unique_source_id, Source, SourceTime,
    },
    utils::{
        ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
        buffer::{add_buffers, clear_buffer, InterleavedBufferMut},
        speed_from_note,
    },
    Error, FilePlaybackOptions, Generator, PlaybackId, PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

/// Wrapped sampler voice types
type SamplerVoiceAmplifiedSource = AmplifiedSource<ChannelMappedSource<PreloadedFileSource>>;
type SamplerVoicePannedSource = PannedSource<SamplerVoiceAmplifiedSource>;
type SamplerVoiceSource = SamplerVoicePannedSource;

// -------------------------------------------------------------------------------------------------

pub(crate) struct SamplerVoice {
    playback_id: Option<PlaybackId>,
    source: SamplerVoiceSource,
    envelope: AhdsrEnvelope,
    release_start_frame: Option<u64>,
}

impl SamplerVoice {
    #[inline(always)]
    /// Is this voice currently playing something?
    pub fn is_active(&self) -> bool {
        self.playback_id.is_some()
    }

    /// Mut access to the voice's panning source.
    #[inline]
    pub fn panned_source_mut(&mut self) -> &mut SamplerVoicePannedSource {
        &mut self.source
    }

    /// Mut access to the voice's volume source.
    #[inline]
    pub fn amplified_source_mut(&mut self) -> &mut SamplerVoiceAmplifiedSource {
        self.source.input_source_mut()
    }

    /// Mut access to the voice's file source.
    #[inline]
    pub fn file_source_mut(&mut self) -> &mut PreloadedFileSource {
        self.source
            .input_source_mut()
            .input_source_mut()
            .input_source_mut()
    }

    /// Stop the voice and start fadeouts .
    pub fn stop(&mut self, envelope_params: &Option<AhdsrParameters>, current_sample_frame: u64) {
        if self.is_active() {
            self.release_start_frame = Some(current_sample_frame);
            if let Some(envelope_params) = envelope_params {
                self.envelope.note_off(envelope_params);
            } else {
                self.file_source_mut().stop();
            }
        }
    }

    /// Stop & reset the voice to finish actual and prepare new playback.
    pub fn reset(&mut self) {
        // reset sources
        if self.is_active() {
            self.file_source_mut().reset();
            self.playback_id = None;
        }
        // reset release start time
        self.release_start_frame = None;
    }

    /// Write source and apply envelope, if set.
    pub fn process(
        &mut self,
        output: &mut [f32],
        channel_count: usize,
        envelope_params: &Option<AhdsrParameters>,
        time: &SourceTime,
    ) -> usize {
        debug_assert!(self.is_active(), "Only active voices need to process");

        // Write source
        let written = self.source.write(output, time);

        // Apply envelope to the voice output
        if let Some(envelope_params) = envelope_params {
            debug_assert!(self.envelope.stage() != AhdsrStage::Idle);
            let mut output = &mut output[..written];
            for frame in output.frames_mut(channel_count) {
                let envelope_value = self.envelope.process(envelope_params) as f32;
                for sample in frame {
                    *sample *= envelope_value;
                }
            }
        }

        // Check if voice finished playback or envelope finished
        if self.source.is_exhausted()
            || (envelope_params.is_some() && self.envelope.stage() == AhdsrStage::Idle)
        {
            self.reset();
        }

        written
    }
}

// -------------------------------------------------------------------------------------------------

/// A simple sampler that plays a single preloaded audio file with an optional AHDSR envelope on
/// a limited set of voices.
pub struct Sampler {
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    playback_id: PlaybackId,
    file_path: Arc<String>,
    voices: Vec<SamplerVoice>,
    envelope_params: Option<AhdsrParameters>,
    attack_param: FloatParameter,
    hold_param: FloatParameter,
    decay_param: FloatParameter,
    sustain_param: FloatParameter,
    release_param: FloatParameter,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    stopping: bool, // True if stop has been called and we are waiting for voices to decay
    stopped: bool,  // True if all voices have decayed after a stop call
    sample_rate: u32,
    channel_count: usize,
    temp_buffer: Vec<f32>,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
    // Parameter IDs
    pub const ATTACK_PARAM_ID: FourCC = FourCC(*b"SATK");
    pub const HOLD_PARAM_ID: FourCC = FourCC(*b"SHLD");
    pub const DECAY_PARAM_ID: FourCC = FourCC(*b"SDCY");
    pub const SUSTAIN_PARAM_ID: FourCC = FourCC(*b"SSTN");
    pub const RELEASE_PARAM_ID: FourCC = FourCC(*b"SREL");

    const MIN_TIME_SEC: f32 = 0.0;
    const MAX_TIME_SEC: f32 = 10.0;

    /// Create a new sampler with the given sample file, optional AHDSR envelope
    /// and the given fixed voice count.
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        envelope_params: Option<AhdsrParameters>,
        playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
        voice_count: usize,
        channel_count: usize,
        sample_rate: u32,
    ) -> Result<Self, Error> {
        let playback_id = unique_source_id();

        // Pre-allocate playback message queue
        const PLAYBACK_MESSAGE_QUEUE_SIZE: usize = 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(PLAYBACK_MESSAGE_QUEUE_SIZE));

        // Load sample file
        let file_path = Arc::new(path.as_ref().to_string_lossy().to_string());
        let sample = PreloadedFileSource::from_file(
            path,
            playback_status_send.clone(),
            Default::default(),
            sample_rate,
        )?;

        // Set voice playback options
        let mut voice_playback_options = FilePlaybackOptions::default();
        if envelope_params.is_none() {
            // just de-click when there's no envelope
            voice_playback_options.fade_out_duration = Some(Duration::from_millis(50));
        } else {
            // use envelope only
            voice_playback_options.fade_out_duration = None;
        }

        // Allocate voices
        let mut voices = Vec::with_capacity(voice_count);
        for _ in 0..voice_count {
            let playback_id = None;

            // Clone sample file source
            let file_source = sample
                .clone(voice_playback_options, sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!("Failed to create sampler voice: {err}"))
                })?;

            // Create wrapped voice source
            let source = {
                // Wrap in ChannelMappedSource to match sampler's channel layout
                let channel_mapped = ChannelMappedSource::new(file_source, channel_count);
                // Wrap in AmplifiedSource for volume control
                let amplified = AmplifiedSource::new(channel_mapped, 1.0);
                // Wrap in PannedSource for panning control
                PannedSource::new(amplified, 0.0)
            };

            // Create envelope state for this voice
            let envelope = AhdsrEnvelope::new();
            let release_start_frame = None;

            voices.push(SamplerVoice {
                playback_id,
                source,
                envelope,
                release_start_frame,
            });
        }

        // Initialize envelope parameters, if any
        let mut envelope_params = envelope_params;
        if let Some(envelope_params) = &mut envelope_params {
            envelope_params
                .set_sample_rate(sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!(
                        "Failed to create envelope parameters for sampler: {err}"
                    ))
                })?;
        }

        // Create AHDSR parameter descriptions
        let attack_param = FloatParameter::new(
            Self::ATTACK_PARAM_ID,
            "Attack",
            Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
            envelope_params
                .as_ref()
                .map(|e| e.attack_time().as_secs_f32().max(Self::MAX_TIME_SEC))
                .unwrap_or(0.01),
        )
        .with_unit("s");
        let hold_param = FloatParameter::new(
            Self::HOLD_PARAM_ID,
            "Hold",
            Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
            envelope_params
                .as_ref()
                .map(|e| e.hold_time().as_secs_f32().max(Self::MAX_TIME_SEC))
                .unwrap_or(1.0),
        )
        .with_unit("s");
        let decay_param = FloatParameter::new(
            Self::DECAY_PARAM_ID,
            "Decay",
            Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
            envelope_params
                .as_ref()
                .map(|e| e.decay_time().as_secs_f32().max(Self::MAX_TIME_SEC))
                .unwrap_or(1.0),
        )
        .with_unit("s");
        let sustain_param = FloatParameter::new(
            Self::SUSTAIN_PARAM_ID, //
            "Sustain",
            0.0..=1.0,
            envelope_params
                .as_ref()
                .map(|e| e.sustain_level() as f32)
                .unwrap_or(1.0),
        );
        let release_param = FloatParameter::new(
            Self::RELEASE_PARAM_ID,
            "Release",
            Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
            envelope_params
                .as_ref()
                .map(|e| e.release_time().as_secs_f32().max(Self::MAX_TIME_SEC))
                .unwrap_or(1.0),
        )
        .with_unit("s");

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
            envelope_params,
            attack_param,
            hold_param,
            decay_param,
            sustain_param,
            release_param,
            stopping,
            stopped,
            sample_rate,
            channel_count,
            temp_buffer,
        })
    }

    /// Access AHDSR envelope time parameters for all voices.
    pub fn envelope_params(&self) -> &Option<AhdsrParameters> {
        &self.envelope_params
    }
    /// Mutably access AHDSR envelope time parameters for all voices.
    pub fn envelope_params_mut(&mut self) -> &mut Option<AhdsrParameters> {
        &mut self.envelope_params
    }

    /// Process pending playback messages from the queue.
    fn process_playback_messages(&mut self, current_sample_frame: u64) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    self.stop(current_sample_frame);
                }
                GeneratorPlaybackMessage::Trigger { event } => match event {
                    GeneratorPlaybackEvent::AllNotesOff => {
                        self.trigger_all_notes_off(current_sample_frame);
                    }
                    GeneratorPlaybackEvent::NoteOn {
                        note_playback_id,
                        note,
                        volume,
                        panning,
                    } => {
                        self.trigger_note_on(note_playback_id, note, volume, panning);
                    }
                    GeneratorPlaybackEvent::NoteOff { note_playback_id } => {
                        self.trigger_note_off(note_playback_id, current_sample_frame);
                    }
                    GeneratorPlaybackEvent::SetSpeed {
                        note_playback_id,
                        speed,
                        glide,
                    } => {
                        self.trigger_set_speed(note_playback_id, speed, glide);
                    }
                    GeneratorPlaybackEvent::SetVolume {
                        note_playback_id,
                        volume,
                    } => {
                        self.trigger_set_volume(note_playback_id, volume);
                    }
                    GeneratorPlaybackEvent::SetPanning {
                        note_playback_id,
                        panning,
                    } => {
                        self.trigger_set_panning(note_playback_id, panning);
                    }
                    GeneratorPlaybackEvent::SetParameter { id, value } => {
                        if let Err(err) = self.apply_parameter_update(id, &value) {
                            log::warn!("Failed to update sampler parameter {id:?}: {err}");
                        }
                    }
                },
            }
        }
    }

    fn stop(&mut self, current_sample_frame: u64) {
        // Mark source as about to stop
        self.stopping = true;
        // Stop all active voices
        for voice in &mut self.voices {
            voice.stop(&self.envelope_params, current_sample_frame);
        }
    }

    /// Immediately trigger a note on (used by event processor)
    fn trigger_note_on(
        &mut self,
        note_playback_id: PlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    ) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        // Allocate a new voice
        let voice_index = self.next_free_voice_index();
        let voice = &mut self.voices[voice_index];
        // Reset a probably recycled file source
        voice.reset();
        // Set initial speed, volume and pan
        let speed = speed_from_note(note);
        voice.file_source_mut().set_speed(speed, None);
        let volume = volume.unwrap_or(1.0);
        voice.amplified_source_mut().set_volume(volume);
        let panning = panning.unwrap_or(0.0);
        voice.panned_source_mut().set_panning(panning);
        // Start envelope
        if let Some(envelope_params) = &self.envelope_params {
            voice.envelope.note_on(envelope_params, 1.0);
        }
        voice.playback_id = Some(note_playback_id);
    }

    fn trigger_note_off(&mut self, playback_id: PlaybackId, current_sample_frame: u64) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            voice.stop(&self.envelope_params, current_sample_frame);
        }
    }

    fn trigger_all_notes_off(&mut self, current_sample_frame: u64) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        for voice in &mut self.voices {
            voice.stop(&self.envelope_params, current_sample_frame);
        }
    }

    fn trigger_set_speed(&mut self, playback_id: PlaybackId, speed: f64, glide: Option<f32>) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            voice.file_source_mut().set_speed(speed, glide);
        }
    }

    fn trigger_set_volume(&mut self, playback_id: PlaybackId, volume: f32) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            voice.amplified_source_mut().set_volume(volume);
        }
    }

    fn trigger_set_panning(&mut self, playback_id: PlaybackId, panning: f32) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            voice.panned_source_mut().set_panning(panning);
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
        let mut oldest_active_playback_id: Option<PlaybackId> = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if self.envelope_params.is_some() && voice.envelope.stage() == AhdsrStage::Release {
                // This voice is in Release stage
                if let Some(release_time) = voice.release_start_frame {
                    if earliest_release_time.is_none_or(|earliest| release_time < earliest) {
                        earliest_release_time = Some(release_time);
                        oldest_active_playback_id = None; // Reset active voices once we found a releasing voice
                        candidate_index = index;
                    }
                }
            } else if earliest_release_time.is_none() {
                // This voice is active (not in Release stage)
                // Only consider if we haven't found a releasing voice yet
                if let Some(playback_id) = voice.playback_id {
                    if oldest_active_playback_id.is_none_or(|oldest| playback_id < oldest) {
                        oldest_active_playback_id = Some(playback_id);
                        candidate_index = index;
                    }
                }
            }
        }
        candidate_index
    }

    fn parameter_descriptors(&self) -> [&dyn ClonableParameter; 5] {
        [
            &self.attack_param,
            &self.hold_param,
            &self.decay_param,
            &self.sustain_param,
            &self.release_param,
        ]
    }

    fn apply_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        let params = self.envelope_params.as_mut().ok_or_else(|| {
            Error::ParameterError("Sampler has no AHDSR envelope configured".to_string())
        })?;

        match id {
            Self::ATTACK_PARAM_ID => {
                let seconds = Self::parameter_update_value(value, &self.attack_param)?;
                params.set_attack_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            Self::HOLD_PARAM_ID => {
                let seconds = Self::parameter_update_value(value, &self.hold_param)?;
                params.set_hold_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            Self::DECAY_PARAM_ID => {
                let seconds = Self::parameter_update_value(value, &self.decay_param)?;
                params.set_decay_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            Self::SUSTAIN_PARAM_ID => {
                let sustain = Self::parameter_update_value(value, &self.sustain_param)? as f64;
                params.set_sustain_level(sustain)?;
            }
            Self::RELEASE_PARAM_ID => {
                let seconds = Self::parameter_update_value(value, &self.release_param)?;
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

// -------------------------------------------------------------------------------------------------

impl Source for Sampler {
    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn is_exhausted(&self) -> bool {
        self.stopped
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // Process playback messages
        self.process_playback_messages(time.pos_in_frames);

        // Return empty handed when exhausted
        if self.stopped {
            return 0;
        }

        // Clear output
        clear_buffer(output);

        // Mix active voices into the output
        let mut active_voices = 0;
        for voice in &mut self.voices {
            if voice.is_active() {
                active_voices += 1;
                assert!(self.temp_buffer.len() >= output.len());
                let mix_buffer = &mut self.temp_buffer[..output.len()];
                clear_buffer(mix_buffer);
                let written =
                    voice.process(mix_buffer, self.channel_count, &self.envelope_params, time);
                add_buffers(&mut output[..written], &mix_buffer[..written]);
            }
        }

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
        if self.envelope_params.is_none() {
            return Vec::new();
        }
        self.parameter_descriptors().into_iter().collect()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: ParameterValueUpdate,
        _time: &SourceTime,
    ) -> Result<(), Error> {
        self.apply_parameter_update(id, &value)
    }
}
