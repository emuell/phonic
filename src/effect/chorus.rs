use std::{any::Any, f64::consts::PI};

use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, FloatParameterValue, Parameter,
        ParameterValueUpdate,
    },
    utils::{
        filter::svf::{SvfFilter, SvfFilterType},
        InterleavedBufferMut,
    },
    Error,
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
    rate: FloatParameterValue,
    phase: FloatParameterValue,
    depth: FloatParameterValue,
    feedback: FloatParameterValue,
    delay: FloatParameterValue,
    wet_mix: FloatParameterValue,
    filter_type: EnumParameterValue<ChorusEffectFilterType>,
    filter_freq: FloatParameterValue,
    filter_resonance: FloatParameterValue,

    // Runtime data
    lfo_range: f64,
    current_phase: f64,

    left_osc: SineWave,
    right_osc: SineWave,

    delay_buffer_left: InterpolatingDelayBuffer,
    delay_buffer_right: InterpolatingDelayBuffer,

    filter_bank_left: SvfFilter,
    filter_bank_right: SvfFilter,
}

impl ChorusEffect {
    pub const EFFECT_NAME: &str = "ChorusEffect";
    pub const RATE_ID: FourCC = FourCC(*b"rate");
    pub const PHASE_ID: FourCC = FourCC(*b"phas");
    pub const DEPTH_ID: FourCC = FourCC(*b"dpth");
    pub const FEEDBACK_ID: FourCC = FourCC(*b"fdbk");
    pub const DELAY_ID: FourCC = FourCC(*b"dlay");
    pub const WET_ID: FourCC = FourCC(*b"wet ");
    pub const FILTER_TYPE_ID: FourCC = FourCC(*b"ftyp");
    pub const FILTER_FREQ_ID: FourCC = FourCC(*b"ffrq");
    pub const FILTER_RESO_ID: FourCC = FourCC(*b"fres");

    const MAX_APPLIED_RANGE_IN_SAMPLES: f64 = 256.0;
    const MAX_APPLIED_DELAY_IN_MS: f64 = 100.0;

    /// Creates a new `ChorusEffect` with default parameter values.
    pub fn new() -> Self {
        Self {
            sample_rate: 0,
            channel_count: 0,

            rate: FloatParameter::new(Self::RATE_ID, "Rate", 0.01..=10.0, 1.0).into(),
            phase: FloatParameter::new(Self::PHASE_ID, "Phase", 0.0..=PI as f32, PI as f32 / 2.0)
                .into(),
            depth: FloatParameter::new(Self::DEPTH_ID, "Depth", 0.0..=1.0, 0.25).into(),
            feedback: FloatParameter::new(Self::FEEDBACK_ID, "Feedback", -1.0..=1.0, 0.5).into(),
            delay: FloatParameter::new(Self::DELAY_ID, "Delay", 0.0..=100.0, 12.0).into(),
            wet_mix: FloatParameter::new(Self::WET_ID, "Wet", 0.0..=1.0, 0.5).into(),
            filter_type: EnumParameter::new(
                Self::FILTER_TYPE_ID,
                "Filter Type",
                ChorusEffectFilterType::Highpass,
            )
            .into(),
            filter_freq: FloatParameter::new(
                Self::FILTER_FREQ_ID,
                "Filter Freq",
                20.0..=22050.0,
                400.0,
            )
            .into(),
            filter_resonance: FloatParameter::new(
                Self::FILTER_RESO_ID,
                "Filter Reso",
                0.0..=1.0,
                0.3,
            )
            .into(),

            lfo_range: 0.0,
            current_phase: 0.0,

            left_osc: SineWave::default(),
            right_osc: SineWave::default(),

            delay_buffer_left: InterpolatingDelayBuffer::default(),
            delay_buffer_right: InterpolatingDelayBuffer::default(),

            filter_bank_left: SvfFilter::default(),
            filter_bank_right: SvfFilter::default(),
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
        chorus.rate.set_value(rate);
        chorus.phase.set_value(phase);
        chorus.depth.set_value(depth);
        chorus.feedback.set_value(feedback);
        chorus.delay.set_value(delay);
        chorus.wet_mix.set_value(wet_mix);
        chorus.filter_type.set_value(filter_type);
        chorus.filter_freq.set_value(filter_freq);
        chorus.filter_resonance.set_value(filter_resonance);
        chorus
    }

    fn update_lfos(&mut self, offset: Option<f64>) {
        if let Some(off) = offset {
            self.current_phase = off * 2.0 * PI;
        }
        self.left_osc
            .set_rate(*self.rate.value() as f64, self.sample_rate);
        self.right_osc
            .set_rate(*self.rate.value() as f64, self.sample_rate);
        self.left_osc.set_phase(self.current_phase);
        self.right_osc
            .set_phase(self.current_phase + *self.phase.value() as f64);
    }

    fn reset(&mut self) {
        self.delay_buffer_left.flush();
        self.delay_buffer_right.flush();
        self.filter_bank_left.reset();
        self.filter_bank_right.reset();
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

    fn parameters(&self) -> Vec<Box<dyn Parameter>> {
        vec![
            Box::new(self.rate.description().clone()),
            Box::new(self.depth.description().clone()),
            Box::new(self.feedback.description().clone()),
            Box::new(self.delay.description().clone()),
            Box::new(self.wet_mix.description().clone()),
            Box::new(self.phase.description().clone()),
            Box::new(self.filter_type.description().clone()),
            Box::new(self.filter_freq.description().clone()),
            Box::new(self.filter_resonance.description().clone()),
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

        self.lfo_range = Self::MAX_APPLIED_RANGE_IN_SAMPLES * (self.sample_rate as f64 / 44100.0);
        let max_depth_in_samples = self.lfo_range.ceil() as usize;
        let max_delay_time_in_samples =
            (Self::MAX_APPLIED_DELAY_IN_MS * self.sample_rate as f64 / 1000.0).ceil() as usize;
        let max_buffer_size = 2 + max_delay_time_in_samples + 2 * max_depth_in_samples + 1;

        self.delay_buffer_left = InterpolatingDelayBuffer::new(max_buffer_size);
        self.delay_buffer_right = InterpolatingDelayBuffer::new(max_buffer_size);

        self.filter_bank_left = SvfFilter::new(
            *self.filter_type.value(),
            sample_rate,
            *self.filter_freq.value(),
            *self.filter_resonance.value() + 0.707,
            1.0,
        )?;
        self.filter_bank_right = SvfFilter::new(
            *self.filter_type.value(),
            sample_rate,
            *self.filter_freq.value(),
            *self.filter_resonance.value() + 0.707,
            1.0,
        )?;

        self.reset();

        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        let delay_ms = *self.delay.value() as f64;
        let depth = *self.depth.value() as f64;
        let feedback = self.feedback.value().clamp(-0.999, 0.999) as f64;
        let wet_amount = *self.wet_mix.value() as f64;
        let dry_amount = 1.0 - wet_amount;

        assert!(self.channel_count == 2);
        for frame in output.as_frames_mut::<2>() {
            let left_input = frame[0] as f64;
            let right_input = frame[1] as f64;

            // Filter the inputs
            let filtered_left = self.filter_bank_left.process_sample(left_input);
            let filtered_right = self.filter_bank_right.process_sample(right_input);

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
        let phase_inc = 2.0 * PI * *self.rate.value() as f64 / self.sample_rate as f64;
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
            Self::FILTER_RESO_ID => self.filter_resonance.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };

        match id {
            Self::RATE_ID | Self::PHASE_ID => self.update_lfos(None),
            Self::FILTER_TYPE_ID => {
                let _ = self
                    .filter_bank_left
                    .coefficients_mut()
                    .set_filter_type(*self.filter_type.value());
                let _ = self
                    .filter_bank_right
                    .coefficients_mut()
                    .set_filter_type(*self.filter_type.value());
            }
            Self::FILTER_FREQ_ID => {
                let _ = self
                    .filter_bank_left
                    .coefficients_mut()
                    .set_cutoff(*self.filter_freq.value());
                let _ = self
                    .filter_bank_right
                    .coefficients_mut()
                    .set_cutoff(*self.filter_freq.value());
            }
            Self::FILTER_RESO_ID => {
                let q = *self.filter_resonance.value() + 0.707;
                let _ = self.filter_bank_left.coefficients_mut().set_q(q);
                let _ = self.filter_bank_right.coefficients_mut().set_q(q);
            }
            _ => {}
        }
        Ok(())
    }
}
