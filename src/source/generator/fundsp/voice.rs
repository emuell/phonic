use fundsp::hacker32::*;

use crate::{
    utils::buffer::{add_buffers, max_abs_sample},
    PlaybackId,
};

// -------------------------------------------------------------------------------------------------

// -60dB as audio silence
const VOICE_SILENCE_THRESHOLD: f32 = 1e-6;
// 2 seconds of silence to ensure full decay
const VOICE_EXHAUSTION_DURATION_SEC: f32 = 2.0;

// -------------------------------------------------------------------------------------------------

/// A single voice that wraps a FunDSP node with control parameters and release state tracking.
pub struct FunDspVoice {
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
    playback_id: Option<PlaybackId>,
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
}

impl FunDspVoice {
    pub fn new(
        audio_unit: Box<dyn AudioUnit>,
        frequency: Shared,
        volume: Shared,
        panning: Shared,
        gate: Shared,
        sample_rate: u32,
    ) -> Self {
        assert!(
            [1, 2].contains(&audio_unit.outputs()),
            "Only mono or stereo voice units are supported"
        );

        let playback_id = None;
        let current_note = None;
        let glide_state = None;
        let is_releasing = false;
        let release_start_frame = None;
        let silence_samples_count = 0;
        let exhaustion_threshold_samples =
            (sample_rate as f32 * VOICE_EXHAUSTION_DURATION_SEC) as usize;
        Self {
            audio_unit,
            frequency,
            volume,
            panning,
            gate,
            playback_id,
            current_note,
            glide_state,
            is_releasing,
            release_start_frame,
            silence_samples_count,
            exhaustion_threshold_samples,
        }
    }

    #[inline(always)]
    pub fn playback_id(&self) -> Option<usize> {
        self.playback_id
    }

    #[inline(always)]
    pub fn is_active(&self) -> bool {
        // A voice is active if it has a playback ID (playing a note)
        // or if it's in its release phase (gate is off but sound is decaying)
        self.playback_id.is_some() || self.is_releasing
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

    /// Start playback on the voice.
    pub fn start(&mut self, playback_id: PlaybackId, note: u8, volume: f32, panning: f32) {
        self.playback_id = Some(playback_id);
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
    }

    /// Stop voice, starting fadeout.
    pub fn stop(&mut self, current_sample_frame: u64) {
        if self.playback_id.is_some() {
            self.gate.set_value(0.0); // Gate off - envelope will fade out
            self.is_releasing = true; // Mark as releasing
            self.silence_samples_count = 0; // Reset silence counter
            self.release_start_frame = Some(current_sample_frame); // Record when release started
        }
    }

    /// Stop voice after a fadeout or brute force kill it.
    pub fn kill(&mut self) {
        self.playback_id = None; // Free up the voice
        self.current_note = None;
        self.is_releasing = false;
        self.silence_samples_count = 0;
        self.frequency.set_value(0.0); // Silence the oscillator by setting frequency to 0.0
        self.release_start_frame = None; // Clear release start time
    }

    pub fn set_speed(&mut self, speed: f64, glide: Option<f32>, sample_rate: u32) {
        if let Some(note) = self.current_note {
            let base_freq = crate::utils::pitch_from_note(note);
            let new_freq = base_freq * speed;

            let glide_duration_samples = if let Some(semitones_per_sec) = glide {
                let current_freq = self.frequency.value() as f64;

                // Calculate the distance in semitones
                let semitone_distance = 12.0 * (new_freq / current_freq).log2();

                // Calculate glide time in seconds: distance / speed
                let glide_time_sec =
                    (semitone_distance.abs() / semitones_per_sec as f64).max(0.0) as f32;

                // Convert to samples
                Some((glide_time_sec * sample_rate as f32) as u32)
            } else {
                None
            };

            self.set_frequency(new_freq, glide_duration_samples);
        }
    }

    pub fn set_frequency(&mut self, freq: f64, glide_duration_samples: Option<u32>) {
        if let Some(duration) = glide_duration_samples {
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

    pub fn process(&mut self, output: &mut [f32]) {
        const BLOCK_SIZE: usize = 64; // Block size for SIMD processing

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
                        if max_abs < VOICE_SILENCE_THRESHOLD {
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

                        if max_abs < VOICE_SILENCE_THRESHOLD {
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
