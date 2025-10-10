use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        FloatParameter, FloatParameterValue, ParameterValueUpdate, SmoothedParameterValue,
    },
    utils::{
        buffer::{copy_buffers, InterleavedBuffer, InterleavedBufferMut},
        db_to_linear,
        dsp::{delay::LookupDelayLine, envelope::EnvelopeFollower},
    },
    ClonableParameter, Error,
};

// -------------------------------------------------------------------------------------------------

/// Stereo compressor effect with limiter mode, lookahead and soft-knee.
///
/// When ratio is above 20.0 it acts as a hard-limiter.
/// Note that the compressor will introduce latency when lookahead is used.
pub struct CompressorEffect {
    // Effect configuration
    sample_rate: u32,
    channel_count: usize,
    // Parameters
    threshold: FloatParameterValue,
    ratio: FloatParameterValue,
    knee_width: FloatParameterValue,
    attack_time: FloatParameterValue,
    release_time: FloatParameterValue,
    makeup_gain: SmoothedParameterValue,
    lookahead_time: FloatParameterValue,
    // Internal state
    envelope_follower: EnvelopeFollower,
    input_buffer: Vec<f32>,
    delay_line: LookupDelayLine<2>,
}

impl CompressorEffect {
    pub const EFFECT_NAME: &str = "CompressorEffect";
    pub const THRESHOLD_ID: FourCC = FourCC(*b"thrs");
    pub const RATIO_ID: FourCC = FourCC(*b"rato");
    pub const ATTACK_ID: FourCC = FourCC(*b"attk");
    pub const RELEASE_ID: FourCC = FourCC(*b"rels");
    pub const MAKEUP_GAIN_ID: FourCC = FourCC(*b"gain");
    pub const KNEE_ID: FourCC = FourCC(*b"knee");
    pub const LOOKAHEAD_ID: FourCC = FourCC(*b"look");

    const DEFAULT_LIMITER_THRESHOLD: f32 = -0.01;

    /// Creates a new `CompressorEffect` with the default parameters.
    pub fn new_compressor() -> Self {
        let ratio_to_string = |value: f32| {
            if value >= 20.0 {
                "LIMIT".to_string()
            } else {
                format!("1:{:.2}", value)
            }
        };
        let string_to_ratio = |string: &str| {
            let trimmed = string.trim();
            if trimmed.eq_ignore_ascii_case("LIMIT") {
                Some(20.0)
            } else if let Some(ratio_str) = trimmed.strip_prefix("1:") {
                ratio_str.parse::<f32>().ok()
            } else {
                trimmed.parse::<f32>().ok()
            }
        };

        Self {
            sample_rate: 0,
            channel_count: 0,
            threshold: FloatParameterValue::from_description(
                FloatParameter::new(
                    Self::THRESHOLD_ID,
                    "Threshold",
                    -60.0..=0.0,
                    -12.0, //
                )
                .with_unit("dB"),
            ),
            ratio: FloatParameterValue::from_description(
                FloatParameter::new(
                    Self::RATIO_ID,
                    "Ratio",
                    1.0..=20.0,
                    8.0, //
                )
                .with_display(ratio_to_string, string_to_ratio),
            ),
            knee_width: FloatParameterValue::from_description(FloatParameter::new(
                Self::KNEE_ID,
                "Knee",
                0.0..=12.0,
                3.0,
            )),
            attack_time: FloatParameterValue::from_description(
                FloatParameter::new(
                    Self::ATTACK_ID,
                    "Attack",
                    0.001..=0.5,
                    0.02, //
                )
                .with_unit("ms"),
            ),
            release_time: FloatParameterValue::from_description(
                FloatParameter::new(
                    Self::RELEASE_ID,
                    "Release",
                    0.1..=2.0,
                    2.0, //
                )
                .with_unit("ms"),
            ),
            makeup_gain: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::MAKEUP_GAIN_ID,
                    "Makeup Gain",
                    -24.0..=24.0,
                    6.0, //
                )
                .with_unit("dB"),
            ),
            lookahead_time: FloatParameterValue::from_description(
                FloatParameter::new(
                    Self::LOOKAHEAD_ID,
                    "Lookahead",
                    0.001..=0.2,
                    0.04, //
                )
                .with_unit("ms"),
            ),
            envelope_follower: EnvelopeFollower::default(),
            input_buffer: Vec::new(),
            delay_line: LookupDelayLine::<2>::default(),
        }
    }

    /// Creates a new `CompressorEffect` with default limiter parameters.
    pub fn new_limiter() -> Self {
        let effect = Self::default();
        let attack = effect.attack_time.description().default_value();
        let release = effect.release_time.description().default_value();
        Self::with_limiter_parameters(Self::DEFAULT_LIMITER_THRESHOLD, attack, release)
    }

    /// Creates a new `CompressorEffect` with the given parameters.
    pub fn with_compressor_parameters(
        threshold: f32,
        ratio: f32,
        knee_width: f32,
        attack_time: f32,
        release_time: f32,
        makeup_gain: f32,
        lookahead_time: f32,
    ) -> Self {
        let mut compressor = Self::default();
        compressor.threshold.set_value(threshold);
        compressor.ratio.set_value(ratio);
        compressor.knee_width.set_value(knee_width);
        compressor.attack_time.set_value(attack_time);
        compressor.release_time.set_value(release_time);
        compressor.makeup_gain.init_value(makeup_gain);
        compressor.lookahead_time.set_value(lookahead_time);
        compressor
    }

    /// Creates a new `CompressorEffect` configured as a limiter.
    pub fn with_limiter_parameters(threshold: f32, attack_time: f32, release_time: f32) -> Self {
        let ratio = 20.0;
        let knee_width = 0.0;
        let makeup_gain = 0.0;
        let lookahead_time = attack_time;
        Self::with_compressor_parameters(
            threshold,
            ratio,
            knee_width,
            attack_time,
            release_time,
            makeup_gain,
            lookahead_time,
        )
    }

    fn update_envelope_follower(&mut self) {
        if self.sample_rate > 0 {
            self.envelope_follower
                .set_attack_time(self.attack_time.value());
            self.envelope_follower
                .set_release_time(self.release_time.value());
        }
    }
}

impl Default for CompressorEffect {
    fn default() -> Self {
        Self::new_compressor()
    }
}

impl Effect for CompressorEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![
            self.threshold.description(),
            self.ratio.description(),
            self.knee_width.description(),
            self.attack_time.description(),
            self.release_time.description(),
            self.makeup_gain.description(),
            self.lookahead_time.description(),
        ]
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

        self.makeup_gain.set_sample_rate(sample_rate);

        self.input_buffer = vec![0.0; max_frames * channel_count];
        self.delay_line = LookupDelayLine::new(sample_rate, self.lookahead_time.value());

        self.envelope_follower = EnvelopeFollower::new(
            sample_rate,
            self.attack_time.value(),
            self.release_time.value(),
        );
        let initial_envelope = if self.ratio.value() >= 20.0 {
            -120.0
        } else {
            0.0
        };
        self.envelope_follower.reset(initial_envelope);

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
            let input_db = if self.ratio.value() >= 20.0 {
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

            // Process envelope
            let envelope = self.envelope_follower.process(input_db);

            // Gain reduction calculation
            let t = self.threshold.value();
            let w = self.knee_width.value();
            let slope = if self.ratio.value() >= 20.0 {
                1.0
            } else {
                1.0 - 1.0 / self.ratio.value()
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
            let makeup_gain = self.makeup_gain.next_value();
            let total_gain_db = makeup_gain - gr_db;
            let total_gain = db_to_linear(total_gain_db);

            out_frame[0] = delayed_frame[0] * total_gain;
            out_frame[1] = delayed_frame[1] * total_gain;
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        let old_lookahead = self.lookahead_time.value();
        match id {
            Self::THRESHOLD_ID => self.threshold.apply_update(value),
            Self::RATIO_ID => self.ratio.apply_update(value),
            Self::KNEE_ID => self.knee_width.apply_update(value),
            Self::ATTACK_ID => self.attack_time.apply_update(value),
            Self::RELEASE_ID => self.release_time.apply_update(value),
            Self::MAKEUP_GAIN_ID => self.makeup_gain.apply_update(value),
            Self::LOOKAHEAD_ID => self.lookahead_time.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        }
        self.update_envelope_follower();
        if self.lookahead_time.value() != old_lookahead && self.sample_rate > 0 {
            self.delay_line = LookupDelayLine::new(self.sample_rate, self.lookahead_time.value());
        }
        Ok(())
    }
}
