use std::{any::Any, f64::consts::PI};

use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        filter::svf::{SvfFilter, SvfFilterCoefficients, SvfFilterType},
        InterleavedBufferMut, LinearSmoothedValue,
    },
    ClonableParameter, Error,
};

// -------------------------------------------------------------------------------------------------

// Simple Sine wave oscillator used as LFO in the chorus effect
#[derive(Debug, Default)]
struct SineWave {
    phase: f64,
    phase_inc: f64,
}

impl SineWave {
    fn set_rate(&mut self, rate: f64, sample_rate: u32) {
        self.phase_inc = 2.0 * PI * rate / sample_rate as f64;
    }

    fn set_phase(&mut self, phase: f64) {
        self.phase = phase;
    }

    // Advances phase and returns new value
    fn move_and_get(&mut self) -> f64 {
        let val = self.phase.sin();
        self.phase += self.phase_inc;
        if self.phase >= 2.0 * PI {
            self.phase -= 2.0 * PI;
        }
        val
    }
}

// -------------------------------------------------------------------------------------------------

// Interpolating Delay Line used in ChorusEffect
#[derive(Debug, Default)]
struct InterpolatingDelayBuffer {
    buffer: Vec<f64>,
    write_pos: usize,
    buffer_mask: usize,
}

impl InterpolatingDelayBuffer {
    fn new(size: usize) -> Self {
        let buffer_size = size.next_power_of_two();
        Self {
            buffer: vec![0.0; buffer_size],
            write_pos: 0,
            buffer_mask: buffer_size - 1,
        }
    }

    fn flush(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    fn process_sample(&mut self, input: f64, feedback: f64, delay_pos: f64) -> f64 {
        let read_pos = self.write_pos as f64 - delay_pos;

        let read_pos_floor = read_pos.floor();
        let fraction = read_pos - read_pos_floor;

        let index1 = read_pos_floor as isize;
        let index2 = index1 + 1;

        let val1 = self.buffer[(index1 as usize) & self.buffer_mask];
        let val2 = self.buffer[(index2 as usize) & self.buffer_mask];

        let output = val1 + (val2 - val1) * fraction;

        self.buffer[self.write_pos] = input + output * feedback;
        self.write_pos = (self.write_pos + 1) & self.buffer_mask;

        output
    }
}

// -------------------------------------------------------------------------------------------------

/// Message type for `ChorusEffect` to change parameters.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum ChorusEffectMessage {
    /// Reset LFO phase and delay lines.
    Reset,
}

impl EffectMessage for ChorusEffectMessage {
    fn effect_name(&self) -> &'static str {
        ChorusEffect::EFFECT_NAME
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// Filter type used in `ChorusEffect`.
pub type ChorusEffectFilterType = SvfFilterType;

// -------------------------------------------------------------------------------------------------

/// A stereo chorus effect with an filtered, interpolated delay-line.
pub struct ChorusEffect {
    sample_rate: u32,
    channel_count: usize,

    // Parameters
    rate: SmoothedParameterValue<LinearSmoothedValue>,
    phase: SmoothedParameterValue<LinearSmoothedValue>,
    depth: SmoothedParameterValue,
    feedback: SmoothedParameterValue,
    delay: SmoothedParameterValue<LinearSmoothedValue>,
    wet_mix: SmoothedParameterValue,
    filter_type: EnumParameterValue<ChorusEffectFilterType>,
    filter_freq: SmoothedParameterValue,
    filter_resonance: SmoothedParameterValue,

    // Runtime data
    lfo_range: f64,
    current_phase: f64,

    left_osc: SineWave,
    right_osc: SineWave,

    delay_buffer_left: InterpolatingDelayBuffer,
    delay_buffer_right: InterpolatingDelayBuffer,

    filter_coefficients: SvfFilterCoefficients,
    filter_left: SvfFilter,
    filter_right: SvfFilter,
}

impl ChorusEffect {
    pub const EFFECT_NAME: &str = "ChorusEffect";
    pub const RATE_ID: FourCC = FourCC(*b"rate");
    pub const PHASE_ID: FourCC = FourCC(*b"phas");
    pub const DEPTH_ID: FourCC = FourCC(*b"dpth");
    pub const FEEDBACK_ID: FourCC = FourCC(*b"fdbk");
    pub const DELAY_ID: FourCC = FourCC(*b"dlay");
    pub const WET_ID: FourCC = FourCC(*b"wet_");
    pub const FILTER_TYPE_ID: FourCC = FourCC(*b"fltt");
    pub const FILTER_FREQ_ID: FourCC = FourCC(*b"fltf");
    pub const FILTER_Q_ID: FourCC = FourCC(*b"fltq");

    const MAX_APPLIED_RANGE_IN_SAMPLES: f64 = 256.0;
    const MAX_APPLIED_DELAY_IN_MS: f64 = 100.0;

    /// Creates a new `ChorusEffect` with default parameter values.
    pub fn new() -> Self {
        let to_string_percent = |v: f32| format!("{:.2}", v * 100.0);
        let from_string_percent = |v: &str| v.parse::<f32>().map(|f| f / 100.0).ok();

        let to_string_degrees = |v: f32| v.to_degrees().round().to_string();
        let from_string_degrees = |v: &str| v.parse::<f32>().map(|f| f.to_radians()).ok();

        Self {
            sample_rate: 0,
            channel_count: 0,

            rate: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::RATE_ID,
                    "Rate",
                    0.01..=10.0,
                    1.0, //
                )
                .with_unit("Hz"),
            ),
            phase: SmoothedParameterValue::from_description(
                FloatParameter::new(Self::PHASE_ID, "Phase", 0.0..=PI as f32, PI as f32 / 2.0)
                    .with_unit("°")
                    .with_display(to_string_degrees, from_string_degrees),
            ),
            depth: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::DEPTH_ID,
                    "Depth",
                    0.0..=1.0,
                    0.25, //
                )
                .with_unit("%")
                .with_display(to_string_percent, from_string_percent),
            ),
            feedback: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::FEEDBACK_ID,
                    "Feedback",
                    -1.0..=1.0,
                    0.5, //
                )
                .with_unit("%")
                .with_display(to_string_percent, from_string_percent),
            ),
            delay: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::DELAY_ID,
                    "Delay",
                    0.0..=100.0,
                    12.0, //
                )
                .with_unit("ms"),
            ),
            wet_mix: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::WET_ID,
                    "Wet",
                    0.0..=1.0,
                    0.5, //
                )
                .with_unit("%")
                .with_display(to_string_percent, from_string_percent),
            ),
            filter_type: EnumParameterValue::from_description(EnumParameter::new(
                Self::FILTER_TYPE_ID,
                "Filter Type",
                ChorusEffectFilterType::Highpass,
            )),
            filter_freq: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::FILTER_FREQ_ID,
                    "Filter Freq",
                    20.0..=22050.0,
                    400.0, //
                )
                .with_unit("Hz"),
            ),
            filter_resonance: SmoothedParameterValue::from_description(FloatParameter::new(
                Self::FILTER_Q_ID,
                "Filter Q",
                0.001..=24.0,
                0.707, //
            )),

            lfo_range: 0.0,
            current_phase: 0.0,

            left_osc: SineWave::default(),
            right_osc: SineWave::default(),

            delay_buffer_left: InterpolatingDelayBuffer::default(),
            delay_buffer_right: InterpolatingDelayBuffer::default(),

            filter_coefficients: SvfFilterCoefficients::default(),
            filter_left: SvfFilter::default(),
            filter_right: SvfFilter::default(),
        }
    }

    /// Creates a new `ChorusEffect` with the given parameters.
    #[allow(clippy::too_many_arguments)]
    pub fn with_parameters(
        rate: f32,
        phase: f32,
        depth: f32,
        feedback: f32,
        delay: f32,
        wet_mix: f32,
        filter_type: ChorusEffectFilterType,
        filter_freq: f32,
        filter_resonance: f32,
    ) -> Self {
        let mut chorus = Self::default();
        chorus.rate.init_value(rate);
        chorus.phase.init_value(phase);
        chorus.depth.init_value(depth);
        chorus.feedback.init_value(feedback);
        chorus.delay.init_value(delay);
        chorus.wet_mix.init_value(wet_mix);
        chorus.filter_type.set_value(filter_type);
        chorus.filter_freq.init_value(filter_freq);
        chorus.filter_resonance.init_value(filter_resonance);
        chorus
    }

    fn update_lfos(&mut self, offset: Option<f64>) {
        if let Some(off) = offset {
            self.current_phase = off * 2.0 * PI;
        }
        let rate = self.rate.next_value() as f64;
        let phase_offset = self.phase.next_value() as f64;
        self.left_osc.set_rate(rate, self.sample_rate);
        self.right_osc.set_rate(rate, self.sample_rate);
        self.left_osc.set_phase(self.current_phase);
        self.right_osc.set_phase(self.current_phase + phase_offset);
    }

    fn reset(&mut self) {
        self.delay_buffer_left.flush();
        self.delay_buffer_right.flush();
        self.filter_left.reset();
        self.filter_right.reset();
        self.rate.init_value(self.rate.target_value());
        self.phase.init_value(self.phase.target_value());
        self.update_lfos(Some(0.0));
    }
}

impl Default for ChorusEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for ChorusEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![
            self.rate.description(),
            self.depth.description(),
            self.feedback.description(),
            self.delay.description(),
            self.wet_mix.description(),
            self.phase.description(),
            self.filter_type.description(),
            self.filter_freq.description(),
            self.filter_resonance.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "ChorusEffect only supports stereo I/O".to_owned(),
            ));
        }

        self.rate.set_sample_rate(sample_rate);
        self.phase.set_sample_rate(sample_rate);
        self.depth.set_sample_rate(sample_rate);
        self.feedback.set_sample_rate(sample_rate);
        self.delay.set_sample_rate(sample_rate);
        self.wet_mix.set_sample_rate(sample_rate);
        self.filter_freq.set_sample_rate(sample_rate);
        self.filter_resonance.set_sample_rate(sample_rate);

        self.lfo_range = Self::MAX_APPLIED_RANGE_IN_SAMPLES * (self.sample_rate as f64 / 44100.0);
        let max_depth_in_samples = self.lfo_range.ceil() as usize;
        let max_delay_time_in_samples =
            (Self::MAX_APPLIED_DELAY_IN_MS * self.sample_rate as f64 / 1000.0).ceil() as usize;
        let max_buffer_size = 2 + max_delay_time_in_samples + 2 * max_depth_in_samples + 1;

        self.delay_buffer_left = InterpolatingDelayBuffer::new(max_buffer_size);
        self.delay_buffer_right = InterpolatingDelayBuffer::new(max_buffer_size);

        self.filter_coefficients = SvfFilterCoefficients::new(
            self.filter_type.value(),
            sample_rate,
            self.filter_freq.target_value(),
            self.filter_resonance.target_value() + 0.707,
            1.0,
        )?;

        self.reset();

        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        let delay_ms = self.delay.next_value() as f64;
        let depth = self.depth.next_value() as f64;
        let feedback = self.feedback.next_value().clamp(-0.999, 0.999) as f64;
        let wet_mix = self.wet_mix.next_value() as f64;
        let wet_amount = wet_mix;
        let dry_amount = 1.0 - wet_mix;

        // ramp and update lfos, if needed
        if self.rate.value_need_ramp() || self.phase.value_need_ramp() {
            self.update_lfos(None);
        }

        assert!(self.channel_count == 2);
        for frame in output.as_frames_mut::<2>() {
            let left_input = frame[0] as f64;
            let right_input = frame[1] as f64;

            // Filter the inputs
            let (filtered_left, filtered_right) =
                if self.filter_freq.value_need_ramp() || self.filter_resonance.value_need_ramp() {
                    let cutoff = self.filter_freq.next_value();
                    let q = self.filter_resonance.next_value() + 0.707;
                    self.filter_coefficients
                        .set(self.filter_type.value(), self.sample_rate, cutoff, q, 0.0)
                        .expect("Failed to set chorus filter parameters");
                    let filtered_left = self
                        .filter_left
                        .process_sample(&self.filter_coefficients, left_input);
                    let filtered_right = self
                        .filter_right
                        .process_sample(&self.filter_coefficients, right_input);
                    (filtered_left, filtered_right)
                } else {
                    let filtered_left = self
                        .filter_left
                        .process_sample(&self.filter_coefficients, left_input);
                    let filtered_right = self
                        .filter_right
                        .process_sample(&self.filter_coefficients, right_input);
                    (filtered_left, filtered_right)
                };

            // Run the LFOs
            let delay_in_samples = delay_ms * self.sample_rate as f64 * 0.001;
            let depth_in_samples = self.lfo_range * depth;

            let left_lfo = self.left_osc.move_and_get();
            let right_lfo = self.right_osc.move_and_get();

            let left_delay_pos = 2.0 + delay_in_samples + (1.0 + left_lfo) * depth_in_samples;
            let right_delay_pos = 2.0 + delay_in_samples + (1.0 + right_lfo) * depth_in_samples;

            // Feed the delays
            let left_output =
                self.delay_buffer_left
                    .process_sample(filtered_left, feedback, left_delay_pos);
            let right_output =
                self.delay_buffer_right
                    .process_sample(filtered_right, feedback, right_delay_pos);

            // Calc the Output
            let out_l = left_input * dry_amount + left_output * wet_amount;
            let out_r = right_input * dry_amount + right_output * wet_amount;

            frame[0] = out_l as f32;
            frame[1] = out_r as f32;
        }

        // Move our LFO offset to keep our oscillators updated when changing the rate or phase
        let phase_inc = 2.0 * PI * self.rate.current_value() as f64 / self.sample_rate as f64;
        self.current_phase += output.len() as f64 / self.channel_count as f64 * phase_inc;
        while self.current_phase >= 2.0 * PI {
            self.current_phase -= 2.0 * PI;
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) -> Result<(), Error> {
        if let Some(message) = message.payload().downcast_ref::<ChorusEffectMessage>() {
            match message {
                ChorusEffectMessage::Reset => self.reset(),
            }
            Ok(())
        } else {
            Err(Error::ParameterError(
                "ChorusEffect: Invalid/unknown message payload".to_owned(),
            ))
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            Self::RATE_ID => self.rate.apply_update(value),
            Self::PHASE_ID => self.phase.apply_update(value),
            Self::DEPTH_ID => self.depth.apply_update(value),
            Self::FEEDBACK_ID => self.feedback.apply_update(value),
            Self::DELAY_ID => self.delay.apply_update(value),
            Self::WET_ID => self.wet_mix.apply_update(value),
            Self::FILTER_TYPE_ID => self.filter_type.apply_update(value),
            Self::FILTER_FREQ_ID => self.filter_freq.apply_update(value),
            Self::FILTER_Q_ID => self.filter_resonance.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };
        match id {
            Self::FILTER_TYPE_ID => self
                .filter_coefficients
                .set_filter_type(self.filter_type.value()),
            _ => Ok(()),
        }
    }
}
