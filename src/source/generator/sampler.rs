use std::{
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;

use crate::{
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
    playback_duration: u64,
    source: SamplerVoiceSource,
    envelope: AhdsrEnvelope,
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
    pub fn stop(&mut self, envelope_params: &Option<AhdsrParameters>) {
        if self.is_active() {
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
        // reset playback count for exhausted voices
        self.playback_duration = 0;
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
        self.playback_duration += written as u64;

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
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    sample_rate: u32,
    channel_count: usize,
    temp_buffer: Vec<f32>,
    stopping: bool,
    stopped: bool,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
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
            let playback_duration = 0;

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

            voices.push(SamplerVoice {
                playback_id,
                playback_duration,
                source,
                envelope,
            });
        }

        let stopping = false;
        let stopped = false;

        // Pre-allocate temp buffer for mixing, using mixer's max sample buffer size
        let temp_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

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

        Ok(Self {
            playback_id,
            playback_message_queue,
            playback_status_send,
            file_path,
            voices,
            envelope_params,
            sample_rate,
            channel_count,
            temp_buffer,
            stopping,
            stopped,
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
    fn process_playback_messages(&mut self) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    self.stop();
                }
                GeneratorPlaybackMessage::Trigger { event } => match event {
                    GeneratorPlaybackEvent::AllNotesOff => {
                        self.trigger_all_notes_off();
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
                        self.trigger_note_off(note_playback_id);
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
                    GeneratorPlaybackEvent::SetParameter { id: _, value: _ } => {
                        unimplemented!()
                    }
                },
            }
        }
    }

    fn stop(&mut self) {
        // Mark source as about to stop
        self.stopping = true;
        // Stop all active voices
        for voice in &mut self.voices {
            voice.stop(&self.envelope_params);
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

    fn trigger_note_off(&mut self, playback_id: PlaybackId) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            voice.stop(&self.envelope_params);
        }
    }

    fn trigger_all_notes_off(&mut self) {
        if self.stopping {
            // Ignore new events when we're about to stop
            return;
        }
        for voice in &mut self.voices {
            voice.stop(&self.envelope_params);
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
        // Try to find a free voice first
        if let Some(index) = self.voices.iter().position(|v| !v.is_active()) {
            index
        } else {
            if self.envelope_params.is_some() {
                // Pick oldest one among all released voices
                let oldest_released_voice = self
                    .voices
                    .iter()
                    .enumerate()
                    .filter(|(_, v1)| v1.envelope.stage() == AhdsrStage::Release)
                    .max_by(|(_, v1), (_, v2)| v1.playback_duration.cmp(&v2.playback_duration));
                if let Some((index, _)) = oldest_released_voice {
                    return index;
                }
            }
            // Pick oldest one among all voices
            let oldest_voice = self
                .voices
                .iter()
                .enumerate()
                .max_by(|(_, v1), (_, v2)| v1.playback_duration.cmp(&v2.playback_duration));
            if let Some((index, _)) = oldest_voice {
                index
            } else {
                0
            }
        }
    }
}

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
        self.process_playback_messages();

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

    fn process_parameter_update(
        &mut self,
        _id: four_cc::FourCC,
        _value: crate::ParameterValueUpdate,
        _time: &SourceTime,
    ) -> Result<(), Error> {
        unimplemented!()
    }
}
