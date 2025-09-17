use std::any::Any;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    utils::{buffer::copy_buffers, db_to_linear, InterleavedBuffer, InterleavedBufferMut},
    Error,
};

// -------------------------------------------------------------------------------------------------

struct DelayLine<const CHANNELS: usize> {
    buffer: Vec<f32>,
    write_pos: usize,
    buffer_mask: usize,
    delay_frames: usize,
    peak_value: f32,
    peak_pos: usize,
}

impl<const CHANNELS: usize> DelayLine<CHANNELS> {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
            write_pos: 0,
            buffer_mask: 0,
            delay_frames: 0,
            peak_value: 0.0,
            peak_pos: 0,
        }
    }

    fn initialize(&mut self, sample_rate: u32, delay_time: f32) {
        self.delay_frames = (delay_time * sample_rate as f32).ceil() as usize;

        if self.delay_frames > 0 {
            let buffer_frames = self.delay_frames.next_power_of_two();
            self.buffer = vec![0.0; buffer_frames * CHANNELS];
            self.buffer_mask = buffer_frames - 1;
        } else {
            self.buffer.clear();
            self.buffer_mask = 0;
        }
        self.write_pos = 0;
        self.peak_value = 0.0;
        self.peak_pos = 0;
    }

    /// Process one frame. Writes the input frame to the delay line and returns the delayed frame.
    fn process(&mut self, input_frame: &[f32; CHANNELS]) -> [f32; CHANNELS] {
        if self.delay_frames == 0 {
            return *input_frame;
        }

        // Read delayed frame from buffer
        let buffer_frames = self.buffer.len() / CHANNELS;
        let read_frame_index =
            (self.write_pos + buffer_frames - self.delay_frames) & self.buffer_mask;
        let read_sample_index = read_frame_index * CHANNELS;

        let mut delayed_frame = [0.0; CHANNELS];
        delayed_frame
            .copy_from_slice(&self.buffer[read_sample_index..read_sample_index + CHANNELS]);

        // Write current frame to buffer
        let write_sample_index = self.write_pos * CHANNELS;
        self.buffer[write_sample_index..write_sample_index + CHANNELS].copy_from_slice(input_frame);

        // update peak
        let peak_expired = self.peak_pos == read_frame_index;
        let new_peak = input_frame
            .iter()
            .fold(0.0f32, |max, &val| max.max(val.abs()));

        if new_peak >= self.peak_value {
            // New frame is the new peak.
            self.peak_value = new_peak;
            self.peak_pos = self.write_pos;
        } else if peak_expired {
            // Old peak expired and new frame is not the peak, so we must rescan.
            self.peak_value = 0.0;
            let buffer_frames = self.buffer.len() / CHANNELS;

            // The lookahead window is the last `delay_frames` that were written.
            // `write_pos` points to the most recently written frame.
            for i in 0..self.delay_frames {
                let frame_index = (self.write_pos + buffer_frames - i) & self.buffer_mask;
                let sample_index = frame_index * CHANNELS;
                let frame_peak = self.buffer[sample_index..sample_index + CHANNELS]
                    .iter()
                    .fold(0.0f32, |max, &val| max.max(val.abs()));
                if frame_peak >= self.peak_value {
                    self.peak_value = frame_peak;
                    self.peak_pos = frame_index;
                }
            }
        }

        // Increment write position
        self.write_pos = (self.write_pos + 1) & self.buffer_mask;

        delayed_frame
    }

    /// Returns the absolute peak value in the delay line.
    fn peak_value(&self) -> f32 {
        self.peak_value
    }
}

// -------------------------------------------------------------------------------------------------

/// Message type for `CompressorEffect` to change parameters.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum CompressorEffectMessage {
    /// Set all compressor parameters at once.
    Init(
        f32, // threshold
        f32, // ratio
        f32, // attack_time
        f32, // release_time
        f32, // makeup_gain
        f32, // knee_width
        f32, // lookahead_time
    ),
    /// Threshold in decibels (dB). Range: -60.0 to 0.0.
    SetThreshold(f32),
    /// Compression ratio. Range: 1.0 to infinity. Ratios >= 20.0 are treated as infinite (limiter).
    SetRatio(f32),
    /// Attack time in seconds. Range: 0.001 to 0.5.
    SetAttack(f32),
    /// Release time in seconds. Range: 0.1 to 2.0.
    SetRelease(f32),
    /// Makeup gain in decibels (dB). Range: -24.0 to 24.0.
    SetMakeupGain(f32),
    /// Knee width in decibels (dB). Range: 0.0 to 12.0.
    SetKnee(f32),
    /// Lookahead time in seconds. Range: 0.001 to 0.2.
    SetLookahead(f32),
}

impl EffectMessage for CompressorEffectMessage {
    fn effect_name(&self) -> &'static str {
        CompressorEffect::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A basic stereo compressor effect with lookahead and soft-knee.
///
/// When ratio is above 20.0 it acts as a hard-limiter.
/// Note that the compressor will introduce latency when lookahead is used.
pub struct CompressorEffect {
    // Effect configuration
    sample_rate: u32,
    channel_count: usize,
    // Parameters
    threshold: f32,
    ratio: f32,
    attack_time: f32,
    release_time: f32,
    makeup_gain: f32,
    knee_width: f32,
    lookahead_time: f32,
    // Internal state
    current_envelope: f32,
    attack_coeff: f32,
    release_coeff: f32,
    input_buffer: Vec<f32>,
    delay_line: DelayLine<2>,
}

impl CompressorEffect {
    const DEFAULT_THRESHOLD: f32 = -12.0;
    const DEFAULT_LIMITER_THRESHOLD: f32 = -0.01;
    const DEFAULT_RATIO: f32 = 8.0;
    const DEFAULT_ATTACK_TIME: f32 = 0.02;
    const DEFAULT_RELEASE_TIME: f32 = 2.0;
    const DEFAULT_MAKEUP_GAIN: f32 = 6.0;
    const DEFAULT_KNEE_WIDTH: f32 = 3.0;
    const DEFAULT_LOOKAHEAD_TIME: f32 = 0.04;

    /// Creates a new `CompressorEffect` with the given parameters.
    pub fn with_parameters(
        threshold: f32,
        ratio: f32,
        attack_time: f32,
        release_time: f32,
        makeup_gain: f32,
        knee_width: f32,
        lookahead_time: f32,
    ) -> Self {
        Self {
            sample_rate: 0,
            channel_count: 0,
            threshold: threshold.clamp(-60.0, 0.0),
            ratio: ratio.clamp(1.0, 20.0),
            attack_time: attack_time.clamp(0.001, 0.5),
            release_time: release_time.clamp(0.1, 2.0),
            makeup_gain: makeup_gain.clamp(-24.0, 24.0),
            knee_width: knee_width.clamp(0.0, 12.0),
            lookahead_time: lookahead_time.clamp(0.001, 0.2),
            current_envelope: 0.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            input_buffer: Vec::new(),
            delay_line: DelayLine::<2>::new(),
        }
    }

    /// Creates a new `CompressorEffect` configured as a limiter.
    pub fn with_limiter_parameters(threshold: f32, attack_time: f32, release_time: f32) -> Self {
        let ratio = 20.0;
        let makeup_gain = 0.0;
        let knee_width = 0.0;
        let lookahead_time = attack_time;

        Self::with_parameters(
            threshold,
            ratio,
            attack_time,
            release_time,
            makeup_gain,
            knee_width,
            lookahead_time,
        )
    }

    pub fn default_limiter() -> Self {
        Self::with_limiter_parameters(
            Self::DEFAULT_LIMITER_THRESHOLD,
            Self::DEFAULT_ATTACK_TIME,
            Self::DEFAULT_RELEASE_TIME,
        )
    }

    fn update_coeffs(&mut self) {
        if self.sample_rate > 0 {
            // Convert time constants to coefficients
            // Using the standard formula: coeff = 1 - exp(-1 / (time * sample_rate))
            self.attack_coeff = if self.attack_time > 0.0 {
                (-1.0 / (self.attack_time * self.sample_rate as f32)).exp()
            } else {
                0.0
            };
            self.release_coeff = if self.release_time > 0.0 {
                (-1.0 / (self.release_time * self.sample_rate as f32)).exp()
            } else {
                0.0
            };
        }
    }
}

impl Default for CompressorEffect {
    fn default() -> Self {
        Self::with_parameters(
            Self::DEFAULT_THRESHOLD,
            Self::DEFAULT_RATIO,
            Self::DEFAULT_ATTACK_TIME,
            Self::DEFAULT_RELEASE_TIME,
            Self::DEFAULT_MAKEUP_GAIN,
            Self::DEFAULT_KNEE_WIDTH,
            Self::DEFAULT_LOOKAHEAD_TIME,
        )
    }
}

impl Effect for CompressorEffect {
    fn name() -> &'static str {
        "CompressorEffect"
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "CompressorEffect only supports stereo I/O".to_string(),
            ));
        }
        self.input_buffer = vec![0.0; max_frames * channel_count];
        self.delay_line.initialize(sample_rate, self.lookahead_time);

        self.current_envelope = if self.ratio >= 20.0 { -120.0 } else { 0.0 };
        self.update_coeffs();

        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        assert!(self.channel_count == 2);

        // Copy input to a temporary buffer because we read from it while writing to `output`
        let input = &mut self.input_buffer[..output.len()];
        copy_buffers(input, output);
        let input_frames = input.as_frames::<2>();

        for (out_frame, in_frame) in output.as_frames_mut::<2>().iter_mut().zip(input_frames) {
            // Get delayed frame from delay line (or original frame if no delay)
            let delayed_frame = self.delay_line.process(in_frame);

            // Envelope detection on current (undelayed) input
            let input_db = if self.ratio >= 20.0 {
                // Limiter mode: use peak from the entire lookahead buffer to prevent overshoots.
                let lookahead_peak = self.delay_line.peak_value();
                if lookahead_peak > 1e-6 {
                    20.0 * lookahead_peak.log10()
                } else {
                    -120.0
                }
            } else {
                // Compressor mode: use peak of current frame.
                let frame_peak = in_frame[0].abs().max(in_frame[1].abs());
                if frame_peak > 1e-6 {
                    20.0 * frame_peak.log10()
                } else {
                    -120.0
                }
            };

            // Envelope follower
            if input_db > self.current_envelope {
                self.current_envelope =
                    input_db + self.attack_coeff * (self.current_envelope - input_db);
            } else {
                self.current_envelope =
                    input_db + self.release_coeff * (self.current_envelope - input_db);
            }

            // Gain reduction calculation
            let envelope = self.current_envelope;
            let t = self.threshold;
            let w = self.knee_width;
            let slope = if self.ratio >= 20.0 {
                1.0
            } else {
                1.0 - 1.0 / self.ratio
            };

            let gr_db = if w > 0.0 && envelope > (t - w / 2.0) && envelope < (t + w / 2.0) {
                // In knee (soft knee)
                let knee_lower = t - w / 2.0;
                let x = (envelope - knee_lower) / w;
                x * x * slope * w / 2.0
            } else if envelope > (t + w / 2.0) {
                // Above knee (hard knee part)
                (envelope - t) * slope
            } else {
                // Below knee
                0.0
            };

            // Apply gain to delayed signal
            let total_gain_db = self.makeup_gain - gr_db;
            let total_gain = db_to_linear(total_gain_db);

            out_frame[0] = delayed_frame[0] * total_gain;
            out_frame[1] = delayed_frame[1] * total_gain;
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<CompressorEffectMessage>() {
            let old_lookahead = self.lookahead_time;
            match message {
                CompressorEffectMessage::Init(
                    threshold,
                    ratio,
                    attack_time,
                    release_time,
                    makeup_gain,
                    knee_width,
                    lookahead_time,
                ) => {
                    self.threshold = *threshold;
                    self.ratio = *ratio;
                    self.attack_time = *attack_time;
                    self.release_time = *release_time;
                    self.makeup_gain = *makeup_gain;
                    self.knee_width = *knee_width;
                    self.lookahead_time = *lookahead_time;
                }
                CompressorEffectMessage::SetThreshold(v) => self.threshold = *v,
                CompressorEffectMessage::SetRatio(v) => self.ratio = *v,
                CompressorEffectMessage::SetAttack(v) => self.attack_time = *v,
                CompressorEffectMessage::SetRelease(v) => self.release_time = *v,
                CompressorEffectMessage::SetMakeupGain(v) => self.makeup_gain = *v,
                CompressorEffectMessage::SetKnee(v) => self.knee_width = *v,
                CompressorEffectMessage::SetLookahead(v) => self.lookahead_time = *v,
            }
            self.update_coeffs();
            if self.lookahead_time != old_lookahead && self.sample_rate > 0 {
                self.delay_line
                    .initialize(self.sample_rate, self.lookahead_time);
            }
        } else {
            log::error!("CompressorEffect: Invalid/unknown message payload");
        }
    }
}
