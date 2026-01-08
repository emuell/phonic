use std::{
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use fundsp::prelude32::*;

use crate::{
    utils::{
        buffer::{add_buffers, max_abs_sample},
        pitch_from_note,
        time::{SampleTime, SampleTimeClock},
    },
    NotePlaybackId, PlaybackStatusContext, PlaybackStatusEvent, SourceTime,
};

// -------------------------------------------------------------------------------------------------

// -60dB as audio silence
const SILENCE_THRESHOLD: f32 = 1e-6;
// 200 ms of silence before killing a released voice
const EXHAUSTION_DURATION: Duration = Duration::from_millis(200);

// -------------------------------------------------------------------------------------------------

/// A single voice that wraps a FunDSP node with control parameters and release state tracking.
pub struct FunDspVoice {
    /// The name of the generator as passed to playback contexts.
    synth_name: Arc<String>,
    /// The voice's audio graph
    audio_unit: Box<dyn AudioUnit>,
    /// Shared variable for frequency control
    frequency: Shared,
    /// Shared variable for volume control
    volume: Shared,
    /// Shared variable for panning control
    panning: Shared,
    /// Shared variable for gate (note on/off)
    gate: Shared,
    /// Currently playing note's playback ID
    note_id: Option<NotePlaybackId>,
    /// Current note
    current_note: Option<u8>,
    /// Glide state for smooth frequency transitions
    glide_state: Option<FunDSPGlideState>,
    /// True if the voice is currently in its release phase (gate is off)
    is_releasing: bool,
    /// The sample frame when the voice started its release phase
    release_start_frame: Option<u64>,
    /// Counter for consecutive samples below the silence threshold during release
    silence_samples_count: usize,
    /// Number of consecutive silent samples required to consider the voice exhausted
    exhaustion_threshold_samples: usize,
    /// Context passed along in PlaybackStatusEvent's
    playback_context: Option<PlaybackStatusContext>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    /// Playback position tracking
    playback_pos: u64,
    playback_pos_emit_rate: Option<SampleTime>,
    playback_pos_sample_time_clock: SampleTimeClock,
    /// Output sample rate
    sample_rate: u32,
}

impl FunDspVoice {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        synth_name: Arc<String>,
        audio_unit: Box<dyn AudioUnit>,
        frequency: Shared,
        volume: Shared,
        panning: Shared,
        gate: Shared,
        playback_pos_emit_rate: Option<Duration>,
        sample_rate: u32,
    ) -> Self {
        assert!(
            [1, 2].contains(&audio_unit.outputs()),
            "Only mono or stereo voice units are supported"
        );

        let note_id = None;
        let current_note = None;
        let glide_state = None;
        let is_releasing = false;
        let release_start_frame = None;
        let silence_samples_count = 0;
        let exhaustion_threshold_samples =
            SampleTimeClock::duration_to_sample_time(EXHAUSTION_DURATION, sample_rate) as usize;
        let playback_status_send = None;
        let playback_context = None;
        let playback_pos = 0;
        let playback_pos_sample_time_clock = SampleTimeClock::new(sample_rate);
        let playback_pos_emit_rate = playback_pos_emit_rate
            .map(|d| SampleTimeClock::duration_to_sample_time(d, sample_rate));

        Self {
            synth_name,
            audio_unit,
            frequency,
            volume,
            panning,
            gate,
            note_id,
            current_note,
            glide_state,
            is_releasing,
            release_start_frame,
            silence_samples_count,
            exhaustion_threshold_samples,
            playback_status_send,
            playback_context,
            playback_pos,
            playback_pos_emit_rate,
            playback_pos_sample_time_clock,
            sample_rate,
        }
    }

    #[inline(always)]
    pub fn note_id(&self) -> Option<NotePlaybackId> {
        self.note_id
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        // A voice is active if it has a playback ID (playing a note)
        // or if it's in its release phase (gate is off but sound is decaying)
        self.note_id.is_some() || self.is_releasing
    }

    /// Returns true if the voice is in its release phase and has been silent for long enough.
    #[inline(always)]
    pub fn is_exhausted(&self) -> bool {
        self.is_releasing && self.silence_samples_count >= self.exhaustion_threshold_samples
    }

    /// Returns true if the voice currently fades out
    #[inline(always)]
    pub fn is_releasing(&self) -> bool {
        self.is_releasing
    }

    /// The sample frame when the voice started its release phase
    #[inline(always)]
    pub fn release_start_frame(&self) -> Option<u64> {
        self.release_start_frame
    }

    /// Set or update our playback status channel.
    pub fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_send = sender;
    }

    /// Start playback on the voice.
    pub fn start(
        &mut self,
        note_id: NotePlaybackId,
        note: u8,
        volume: f32,
        panning: f32,
        context: Option<PlaybackStatusContext>,
    ) {
        self.note_id = Some(note_id);
        let freq = crate::utils::pitch_from_note(note);
        self.frequency.set_value(freq as f32);
        self.volume.set_value(volume);
        self.panning.set_value(panning);
        self.gate.set_value(1.0); // Gate on
        self.current_note = Some(note);
        self.glide_state = None; // Clear any ongoing glide
        self.is_releasing = false; // Not releasing when a new note starts
        self.silence_samples_count = 0; // Reset silence counter
        self.release_start_frame = None; // Not releasing when a new note starts
        self.audio_unit.reset(); // Reset envelope and oscillator phase
        self.playback_pos = 0; // Reset position and context
        self.playback_context = context;
    }

    /// Stop voice, starting fadeout.
    pub fn stop(&mut self, current_sample_frame: u64) {
        if self.note_id.is_some() {
            self.gate.set_value(0.0); // Gate off - envelope will fade out
            self.is_releasing = true; // Mark as releasing
            self.silence_samples_count = 0; // Reset silence counter
            self.release_start_frame = Some(current_sample_frame); // Record when release started
        }
    }

    /// Stop voice after a fadeout or brute force kill it.
    pub fn kill(&mut self) {
        // Send stopped event for killed voice
        if self.note_id.is_some() {
            self.send_stopped_event(self.is_exhausted());
        }
        self.note_id = None; // Free up the voice
        self.current_note = None;
        self.is_releasing = false;
        self.silence_samples_count = 0;
        self.frequency.set_value(0.0); // Silence the oscillator by setting frequency to 0.0
        self.release_start_frame = None; // Clear release start time
        self.playback_context = None; // Reset position and context
        self.playback_pos = 0;
    }

    pub fn set_speed(&mut self, speed: f64, glide: Option<f32>, sample_rate: u32) {
        if let Some(note) = self.current_note {
            let base_freq = pitch_from_note(note);
            let new_freq = base_freq * speed;
            let glide_duration_samples = if let Some(semitones_per_sec) = glide {
                let current_freq = self.frequency.value() as f64;
                // Calculate the distance in semitones
                let semitone_distance = 12.0 * (new_freq / current_freq).log2();
                if semitone_distance.abs() > 0.0 && semitones_per_sec > 0.0 {
                    // Calculate glide time in seconds: distance / speed
                    let glide_time_sec =
                        (semitone_distance.abs() / semitones_per_sec as f64).max(0.0) as f32;
                    // Convert to samples
                    let glide_time_samples = (glide_time_sec * sample_rate as f32) as u32;
                    Some(glide_time_samples)
                } else {
                    None
                }
            } else {
                None
            };
            self.set_frequency(new_freq, glide_duration_samples);
        }
    }

    pub fn set_frequency(&mut self, freq: f64, glide_duration_samples: Option<u32>) {
        if let Some(duration) = glide_duration_samples.filter(|g| *g > 0) {
            let current_freq = self.frequency.value();
            self.glide_state = Some(FunDSPGlideState::new(current_freq, freq as f32, duration));
        } else {
            self.frequency.set_value(freq as f32);
            self.glide_state = None;
        }
    }

    pub fn set_volume(&mut self, vol: f32) {
        self.volume.set_value(vol);
    }

    pub fn set_panning(&mut self, panning: f32) {
        self.panning.set_value(panning);
    }

    pub fn process(&mut self, output: &mut [f32], time: &SourceTime) {
        // Send playback start events
        if self.playback_pos == 0 {
            let is_start_event = true;
            self.send_position_event(time, is_start_event);
        }

        // Process in SIMD friendly blocks as long as possible
        const BLOCK_SIZE: usize = 64;
        let frame_count = output.len() / self.audio_unit.outputs();
        for block_start in (0..frame_count).step_by(BLOCK_SIZE) {
            let block_end = std::cmp::min(block_start + BLOCK_SIZE, frame_count);
            let block_len = block_end - block_start;
            // Update glide state for the entire block if active
            if self.glide_state.is_some() {
                self.update_glide(block_len);
            }
            match self.audio_unit.outputs() {
                1 => {
                    // Mono processing
                    let input_buffer = BufferArray::<U1>::new();
                    let mut output_buffer = BufferArray::<U1>::new();
                    self.audio_unit.process(
                        block_len,
                        &input_buffer.buffer_ref(),
                        &mut output_buffer.buffer_mut(),
                    );
                    // Check for silence in the entire block using SIMD if voice is releasing
                    if self.is_releasing {
                        let max_abs = max_abs_sample(&output_buffer.channel_f32(0)[..block_len]);
                        if max_abs < SILENCE_THRESHOLD {
                            self.silence_samples_count =
                                self.silence_samples_count.saturating_add(block_len);
                        } else {
                            self.silence_samples_count = 0;
                        }
                    }
                    // Extract samples from the output buffer and mix into the main output.
                    add_buffers(
                        &mut output[block_start..block_start + block_len],
                        &output_buffer.channel_f32(0)[..block_len],
                    );
                }
                2 => {
                    // Stereo processing
                    let input_buffer = BufferArray::<U2>::new();
                    let mut output_buffer = BufferArray::<U2>::new();
                    self.audio_unit.process(
                        block_len,
                        &input_buffer.buffer_ref(),
                        &mut output_buffer.buffer_mut(),
                    );
                    // Check for silence in both channels
                    if self.is_releasing {
                        let max_abs_left =
                            max_abs_sample(&output_buffer.channel_f32(0)[..block_len]);
                        let max_abs_right =
                            max_abs_sample(&output_buffer.channel_f32(1)[..block_len]);
                        let max_abs = max_abs_left.max(max_abs_right);

                        if max_abs < SILENCE_THRESHOLD {
                            self.silence_samples_count =
                                self.silence_samples_count.saturating_add(block_len);
                        } else {
                            self.silence_samples_count = 0;
                        }
                    }
                    // Mix stereo output to output buffer.
                    let start_offset = block_start * 2;
                    for index in 0..block_len {
                        let left = output_buffer.at_f32(0, index);
                        let right = output_buffer.at_f32(1, index);
                        let frame_start = start_offset + index * 2;
                        output[frame_start] += left;
                        output[frame_start + 1] += right;
                    }
                }
                _ => unreachable!("Expected mono or stereo funDSP voices"),
            }
        }

        // Update playback position
        let frames_processed = output.len() / self.audio_unit.outputs();
        self.playback_pos += frames_processed as u64;

        // Send position event if needed
        let is_start_event = false;
        self.send_position_event(time, is_start_event);

        // kill the voice when it got exhausted
        if self.is_exhausted() {
            self.kill();
        }
    }

    fn update_glide(&mut self, samples_count: usize) {
        if let Some(glide) = &mut self.glide_state {
            if let Some(freq) = glide.update(samples_count) {
                self.frequency.set_value(freq);
            } else {
                // Glide finished
                self.glide_state = None;
            }
        }
    }

    fn should_report_pos(&self, time: &SourceTime, is_start_event: bool) -> bool {
        if let Some(emit_rate) = self.playback_pos_emit_rate {
            is_start_event
                || self
                    .playback_pos_sample_time_clock
                    .elapsed(time.pos_in_frames)
                    >= emit_rate
        } else {
            false
        }
    }

    fn samples_to_duration(&self, samples: u64) -> std::time::Duration {
        let frames = samples / self.audio_unit.outputs() as u64;
        let seconds = frames as f64 / self.sample_rate as f64;
        std::time::Duration::from_secs_f64(seconds)
    }

    fn send_position_event(&mut self, time: &SourceTime, is_start_event: bool) {
        if let Some(sender) = &self.playback_status_send {
            if self.should_report_pos(time, is_start_event) {
                self.playback_pos_sample_time_clock
                    .reset(time.pos_in_frames);
                if let Some(note_id) = self.note_id {
                    if let Err(err) = sender.try_send(PlaybackStatusEvent::Position {
                        id: note_id,
                        context: self.playback_context.clone(),
                        path: Arc::clone(&self.synth_name),
                        position: self.samples_to_duration(self.playback_pos),
                    }) {
                        log::warn!("Failed to send fundsp voice position event: {err}")
                    }
                }
            }
        }
    }

    fn send_stopped_event(&mut self, exhausted: bool) {
        if let Some(sender) = &self.playback_status_send {
            if let Some(note_id) = self.note_id {
                if let Err(err) = sender.send(PlaybackStatusEvent::Stopped {
                    id: note_id,
                    context: self.playback_context.clone(),
                    path: Arc::clone(&self.synth_name),
                    exhausted,
                }) {
                    log::warn!("Failed to send fundsp voice stopped event: {err}");
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// State for frequency gliding to mimic the file source's glide behavior.
struct FunDSPGlideState {
    /// Starting frequency
    start_freq: f32,
    /// Target frequency
    target_freq: f32,
    /// Glide duration in samples
    duration_samples: usize,
    /// Current sample position in glide
    current_sample: usize,
}

impl FunDSPGlideState {
    fn new(start_freq: f32, target_freq: f32, duration_samples: u32) -> Self {
        debug_assert!(
            duration_samples > 0,
            "Invalid duration for a note glide, duration must be > 0"
        );
        let duration_samples = duration_samples as usize;
        let current_sample = 0;
        Self {
            start_freq,
            target_freq,
            duration_samples,
            current_sample,
        }
    }

    /// Update glide state and return current frequency, or None if glide is finished.
    /// Advances the glide by `samples_count` samples.
    fn update(&mut self, samples_count: usize) -> Option<f32> {
        // Calculate the frequency at the end of the advance
        if self.current_sample >= self.duration_samples {
            self.current_sample = self.duration_samples;
            None // Glide finished
        } else if self.current_sample + samples_count >= self.duration_samples {
            self.current_sample = self.duration_samples;
            Some(self.target_freq) // Glide finishes in this slice
        } else {
            // Linear interpolation
            let t = self.current_sample as f32 / self.duration_samples as f32;
            let freq = self.start_freq + (self.target_freq - self.start_freq) * t;
            self.current_sample += samples_count;
            Some(freq)
        }
    }
}
