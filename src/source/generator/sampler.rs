use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

use crate::{
    source::{
        amplified::{AmplifiedSource, AmplifiedSourceMessage},
        file::{preloaded::PreloadedFileSource, FilePlaybackMessage},
        generator::GeneratorPlaybackMessage,
        mapped::ChannelMappedSource,
        mixed::MixedSource,
        panned::{PannedSource, PannedSourceMessage},
        Source, SourceTime,
    },
    utils::buffer::{add_buffers, clear_buffer},
    utils::speed_from_note,
    Error, FilePlaybackOptions, FileSource, Generator, PlaybackId,
};

// -------------------------------------------------------------------------------------------------

struct SamplerVoice {
    /// The voice's playback source (wrapped in conversion, amplification, and panning)
    source: Box<dyn Source>,
    /// Message queue for speed changes on the underlying file source
    file_source_queue: Arc<ArrayQueue<FilePlaybackMessage>>,
    /// Message queue for volume changes
    volume_queue: Arc<ArrayQueue<AmplifiedSourceMessage>>,
    /// Message queue for panning changes
    panning_queue: Arc<ArrayQueue<PannedSourceMessage>>,
    /// Currently playing note's playback ID
    playback_id: Option<PlaybackId>,
}

impl SamplerVoice {
    #[inline(always)]
    /// Is this voice currently playing something?
    fn is_active(&self) -> bool {
        self.playback_id.is_some()
    }
}

// -------------------------------------------------------------------------------------------------

/// A simple sampler that plays a single preloaded audio file.
pub struct Sampler {
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    voices: Vec<SamplerVoice>,
    last_voice_index: Option<usize>,
    stopped: bool,
    sample_rate: u32,
    channel_count: usize,
    temp_buffer: Vec<f32>,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
    /// Create a new sampler with the given sample and voice count.
    ///
    /// Note: This allocates voices and is NOT real-time safe.
    /// Create the sampler before adding it to the player.
    pub fn new(
        sample: PreloadedFileSource,
        voice_count: usize,
        sample_rate: u32,
        channel_count: usize,
        fadeout_duration: Option<std::time::Duration>,
    ) -> Result<Self, Error> {
        // Pre-allocate playback message queue
        const PLAYBACK_MESSAGE_QUEUE_SIZE: usize = 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(PLAYBACK_MESSAGE_QUEUE_SIZE));

        // Set sample playback options
        let mut voice_playback_options = FilePlaybackOptions::default();
        if let Some(duration) = fadeout_duration {
            voice_playback_options.fade_out_duration = Some(duration);
        } else {
            voice_playback_options.fade_out_duration = Some(std::time::Duration::from_millis(50));
            // just declick
        }

        // Pre-allocate voices
        let mut voices = Vec::with_capacity(voice_count);
        for _ in 0..voice_count {
            let voice_source =
                sample
                    .clone(voice_playback_options, sample_rate)
                    .map_err(|err| {
                        Error::ParameterError(format!("Failed to create sampler voice: {err}"))
                    })?;

            // Get the speed message queue from the file source
            let file_source_queue = voice_source.playback_message_queue();

            // Wrap in ChannelMappedSource to match sampler's channel layout
            let channel_mapped = ChannelMappedSource::new(voice_source, channel_count);

            // Wrap in AmplifiedSource for volume control
            let amplified = AmplifiedSource::new(channel_mapped, 1.0);
            let volume_queue = amplified.message_queue();

            // Wrap in PannedSource for panning control
            let panned = PannedSource::new(amplified, 0.0);
            let panning_queue = panned.message_queue();

            // wrap final source into a box
            let source = Box::new(panned) as Box<dyn Source>;

            voices.push(SamplerVoice {
                source,
                file_source_queue,
                volume_queue,
                panning_queue,
                playback_id: None,
            });
        }

        let last_voice_index = None;
        let stopped = false;

        // Pre-allocate temp buffer for mixing
        let temp_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

        Ok(Self {
            playback_message_queue,
            voices,
            last_voice_index,
            stopped,
            sample_rate,
            channel_count,
            temp_buffer,
        })
    }

    /// Find a free voice or steal the oldest one.
    /// Returns the index of the allocated voice.
    fn allocate_voice(&mut self) -> usize {
        // Try to find a free voice first
        if let Some(index) = self.voices.iter().position(|v| !v.is_active()) {
            // Remember this voice as the last triggered one
            self.last_voice_index = Some(index);
            return index;
        }
        // No free voices - steal the first one (simple voice stealing)
        0
    }

    /// Immediately trigger a note on (used by event processor)
    fn trigger_note_on(
        &mut self,
        note_playback_id: PlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
    ) {
        let voice_index = self.allocate_voice();
        let voice = &mut self.voices[voice_index];

        // Reset the file source via message queue
        let _ = voice.file_source_queue.push(FilePlaybackMessage::Reset);

        // Set volume via message queue
        let final_volume = volume.unwrap_or(1.0);
        let _ = voice
            .volume_queue
            .push(AmplifiedSourceMessage::SetVolume(final_volume));

        // Set speed via message queue
        let speed = speed_from_note(note);
        let _ = voice
            .file_source_queue
            .push(FilePlaybackMessage::SetSpeed(speed, None));

        // Set panning via message queue if provided
        if let Some(panning) = panning {
            let _ = voice
                .panning_queue
                .push(PannedSourceMessage::SetPanning(panning));
        }

        voice.playback_id = Some(note_playback_id);
    }

    fn trigger_note_off(&mut self, playback_id: PlaybackId) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            // send stop command: will collect the voice when it finished playing
            let _ = voice
                .file_source_queue
                .force_push(FilePlaybackMessage::Stop);
        }
    }

    fn trigger_set_speed(&mut self, playback_id: PlaybackId, speed: f64, glide: Option<f32>) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            let _ = voice
                .file_source_queue
                .push(FilePlaybackMessage::SetSpeed(speed, glide));
        }
    }

    fn trigger_set_volume(&mut self, playback_id: PlaybackId, volume: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            let _ = voice
                .volume_queue
                .push(AmplifiedSourceMessage::SetVolume(volume));
        }
    }

    fn trigger_set_panning(&mut self, playback_id: PlaybackId, panning: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.playback_id == Some(playback_id))
        {
            let _ = voice
                .panning_queue
                .push(PannedSourceMessage::SetPanning(panning));
        }
    }

    /// Process pending playback messages from the queue.
    fn process_playback_messages(&mut self) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    // mark source as exhausted
                    self.stopped = true;
                    // Stop all active voices
                    for voice in &mut self.voices {
                        if voice.is_active() {
                            voice.playback_id = None;
                        }
                    }
                }
                GeneratorPlaybackMessage::AllNotesOff => {
                    // Stop all active voices but don't mark the sampler as stopped
                    for voice in &mut self.voices {
                        if voice.is_active() {
                            // Send stop command to trigger fadeout
                            let _ = voice
                                .file_source_queue
                                .force_push(FilePlaybackMessage::Stop);
                        }
                    }
                }
                GeneratorPlaybackMessage::NoteOn {
                    note_playback_id,
                    note,
                    volume,
                    panning,
                } => {
                    self.trigger_note_on(note_playback_id, note, volume, panning);
                }
                GeneratorPlaybackMessage::NoteOff { note_playback_id } => {
                    self.trigger_note_off(note_playback_id);
                }
                GeneratorPlaybackMessage::SetSpeed {
                    note_playback_id,
                    speed,
                    glide,
                } => {
                    self.trigger_set_speed(note_playback_id, speed, glide);
                }
                GeneratorPlaybackMessage::SetVolume {
                    note_playback_id,
                    volume,
                } => {
                    self.trigger_set_volume(note_playback_id, volume);
                }
                GeneratorPlaybackMessage::SetPanning {
                    note_playback_id,
                    panning,
                } => {
                    self.trigger_set_panning(note_playback_id, panning);
                }
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
        // Process playback messages first
        self.process_playback_messages();

        // If stopped, there's nothing to calc
        if self.stopped {
            return 0;
        }

        // Clear output
        clear_buffer(output);

        // Mix all active voices int the output
        for voice in &mut self.voices {
            if voice.is_active() {
                // Prepare temp buffer
                assert!(self.temp_buffer.len() >= output.len());
                let temp_buffer = &mut self.temp_buffer[..output.len()];
                clear_buffer(temp_buffer);

                // Run voice in temp buffer
                let written = voice.source.write(temp_buffer, time);

                // Mix into output
                add_buffers(&mut output[..written], &temp_buffer[..written]);

                // Check if voice finished playback
                if voice.source.is_exhausted() {
                    voice.playback_id = None;
                }
            }
        }

        // we've cleared the entire buffer, so return all
        output.len()
    }
}

impl Generator for Sampler {
    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        self.playback_message_queue.clone()
    }
}
