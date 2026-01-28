use std::{any::Any, f64::consts::PI};

use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        dsp::{
            delay::InterpolatedDelayLine,
            filters::biquad::{BiquadFilter, BiquadFilterCoefficients, BiquadFilterType},
            lfo::{Lfo, LfoWaveform},
        },
        smoothing::LinearSmoothedValue,
    },
    Error, Parameter, ParameterScaling,
};

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
#[derive(
    Default, Clone, Copy, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
#[allow(unused)]
pub enum ChorusEffectFilterType {
    #[default]
    None,
    Lowpass,
    Bandpass,
    Bandstop,
    Highpass,
}

impl From<ChorusEffectFilterType> for BiquadFilterType {
    fn from(val: ChorusEffectFilterType) -> Self {
        match val {
            ChorusEffectFilterType::None => BiquadFilterType::Allpass,
            ChorusEffectFilterType::Lowpass => BiquadFilterType::Lowpass,
            ChorusEffectFilterType::Bandpass => BiquadFilterType::Bandpass,
            ChorusEffectFilterType::Bandstop => BiquadFilterType::Notch,
            ChorusEffectFilterType::Highpass => BiquadFilterType::Highpass,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Stereo chorus effect with an filtered, interpolated delay-line.
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
    lfo_range: f32,
    current_phase: f64,
    left_osc: Lfo,
    right_osc: Lfo,
    delay_buffer_left: InterpolatedDelayLine<1>,
    delay_buffer_right: InterpolatedDelayLine<1>,
    filter_coefficients: BiquadFilterCoefficients,
    filter_left: BiquadFilter,
    filter_right: BiquadFilter,
    // Block processing buffers (persistent across calls)
    delay_block: Vec<f32>,
    depth_block: Vec<f32>,
    feedback_block: Vec<f32>,
    wet_mix_block: Vec<f32>,
    filter_freq_block: Vec<f32>,
    filter_resonance_block: Vec<f32>,
    block_position: usize,
    current_block_size: usize,
}

impl ChorusEffect {
    pub const EFFECT_NAME: &str = "Chorus";

    pub const RATE: FloatParameter = FloatParameter::new(
        FourCC(*b"rate"),
        "Rate",
        0.01..=10.0,
        1.0, //
    )
    .with_unit("Hz");
    pub const PHASE: FloatParameter = FloatParameter::new(
        FourCC(*b"phas"), //
        "Phase",
        0.0..=PI as f32,
        PI as f32 / 2.0,
    )
    .with_unit("°");
    pub const DEPTH: FloatParameter = FloatParameter::new(
        FourCC(*b"dpth"),
        "Depth",
        0.0..=1.0,
        0.25, //
    )
    .with_unit("%");
    pub const FEEDBACK: FloatParameter = FloatParameter::new(
        FourCC(*b"fdbk"),
        "Feedback",
        -1.0..=1.0,
        0.5, //
    )
    .with_unit("%");
    pub const DELAY: FloatParameter = FloatParameter::new(
        FourCC(*b"dlay"),
        "Delay",
        0.0..=100.0,
        12.0, //
    )
    .with_unit("ms");
    pub const WET_MIX: FloatParameter = FloatParameter::new(
        FourCC(*b"wet_"),
        "Wet",
        0.0..=1.0,
        0.5, //
    )
    .with_unit("%");
    pub const FILTER_TYPE: EnumParameter = EnumParameter::new(
        FourCC(*b"fltt"),
        "Filter Type",
        ChorusEffectFilterType::VARIANTS,
        0,
    );
    pub const FILTER_FREQ: FloatParameter = FloatParameter::new(
        FourCC(*b"fltf"),
        "Filter Freq",
        20.0..=20000.0,
        400.0, //
    )
    .with_unit("Hz")
    .with_scaling(ParameterScaling::Exponential(2.5));
    pub const FILTER_Q: FloatParameter = FloatParameter::new(
        FourCC(*b"fltq"),
        "Filter Q",
        0.001..=4.0,
        0.707, //
    );

    const MAX_APPLIED_RANGE_IN_SAMPLES: f32 = 256.0;
    const MAX_APPLIED_DELAY_IN_MS: f32 = 100.0;

    /// Creates a new `ChorusEffect` with default parameter values.
    pub fn new() -> Self {
        let to_string_percent = |v: f32| format!("{:.2}", v * 100.0);
        let from_string_percent = |v: &str| v.parse::<f32>().map(|f| f / 100.0).ok();

        let to_string_degrees = |v: f32| v.to_degrees().round().to_string();
        let from_string_degrees = |v: &str| v.parse::<f32>().map(|f| f.to_radians()).ok();

        Self {
            sample_rate: 0,
            channel_count: 0,

            rate: SmoothedParameterValue::from_description(Self::RATE) //
                .with_smoother(LinearSmoothedValue::default().with_step(0.005)),
            phase: SmoothedParameterValue::from_description(
                Self::PHASE
                    .clone()
                    .with_display(to_string_degrees, from_string_degrees),
            )
            .with_smoother(LinearSmoothedValue::default().with_step(0.001)),
            depth: SmoothedParameterValue::from_description(
                Self::DEPTH
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            feedback: SmoothedParameterValue::from_description(
                Self::FEEDBACK
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            delay: SmoothedParameterValue::from_description(Self::DELAY)
                .with_smoother(LinearSmoothedValue::default().with_step(0.01)),
            wet_mix: SmoothedParameterValue::from_description(
                Self::WET_MIX
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            filter_type: EnumParameterValue::from_description(Self::FILTER_TYPE),
            filter_freq: SmoothedParameterValue::from_description(Self::FILTER_FREQ),
            filter_resonance: SmoothedParameterValue::from_description(Self::FILTER_Q),

            lfo_range: 0.0,
            current_phase: 0.0,

            left_osc: Lfo::default(),
            right_osc: Lfo::default(),

            delay_buffer_left: InterpolatedDelayLine::default(),
            delay_buffer_right: InterpolatedDelayLine::default(),

            filter_coefficients: BiquadFilterCoefficients::default(),
            filter_left: BiquadFilter::default(),
            filter_right: BiquadFilter::default(),

            // Initialize block buffers (max 64 samples per block)
            delay_block: vec![0.0; 64],
            depth_block: vec![0.0; 64],
            feedback_block: vec![0.0; 64],
            wet_mix_block: vec![0.0; 64],
            filter_freq_block: vec![0.0; 64],
            filter_resonance_block: vec![0.0; 64],
            block_position: 0,
            current_block_size: 0,
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
        let mut chorus = Self::new(); // Use new() instead of default() to ensure proper initialization
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

    fn reset(&mut self) {
        self.delay_buffer_left.flush();
        self.delay_buffer_right.flush();
        self.filter_left.reset();
        self.filter_right.reset();
        self.rate.init_value(self.rate.target_value());
        self.phase.init_value(self.phase.target_value());
        self.current_phase = 0.0;
        self.reset_lfos();
    }

    fn reset_lfos(&mut self) {
        let rate = self.rate.current_value() as f64;
        self.left_osc = Lfo::new(self.sample_rate, rate, LfoWaveform::Sine);
        self.right_osc = Lfo::new(self.sample_rate, rate, LfoWaveform::Sine);
        let phase_offset = self.phase.current_value() as f64;
        self.left_osc.set_phase_degrees(self.current_phase as f32);
        self.right_osc
            .set_phase_degrees((self.current_phase + phase_offset) as f32);
    }

    fn update_lfos(&mut self) {
        let rate = self.rate.next_value() as f64;
        self.left_osc.set_rate(self.sample_rate, rate);
        self.right_osc.set_rate(self.sample_rate, rate);
        let phase_offset = self.phase.next_value() as f64;
        self.left_osc.set_phase_degrees(self.current_phase as f32);
        self.right_osc
            .set_phase_degrees((self.current_phase + phase_offset) as f32);
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

    fn weight(&self) -> usize {
        3
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
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

        self.lfo_range = Self::MAX_APPLIED_RANGE_IN_SAMPLES * (self.sample_rate as f32 / 44100.0);
        let max_depth_in_samples = self.lfo_range.ceil() as usize;
        let max_delay_time_in_samples =
            (Self::MAX_APPLIED_DELAY_IN_MS * self.sample_rate as f32 / 1000.0).ceil() as usize;
        let max_buffer_size = 2 + max_delay_time_in_samples + 2 * max_depth_in_samples + 1;

        self.delay_buffer_left = InterpolatedDelayLine::new(max_buffer_size);
        self.delay_buffer_right = InterpolatedDelayLine::new(max_buffer_size);

        self.filter_coefficients = BiquadFilterCoefficients::new(
            self.filter_type.value().into(),
            sample_rate,
            self.filter_freq.target_value(),
            self.filter_resonance.target_value() + 0.707,
            1.0,
        )?;

        self.reset();

        Ok(())
    }

    fn process(&mut self, output: &mut [f32], _time: &EffectTime) {
        assert!(self.channel_count == 2);

        const BLOCK_SIZE: usize = 64;
        let frame_count = output.len() / self.channel_count;
        let mut frame_idx = 0;

        while frame_idx < frame_count {
            // Check if we need to compute new parameter block
            if self.block_position >= self.current_block_size {
                let remaining = frame_count - frame_idx;
                let chunk_size = remaining.min(BLOCK_SIZE);

                // Process parameter smoothing for this chunk
                self.delay.process_block(&mut self.delay_block, chunk_size);
                self.depth.process_block(&mut self.depth_block, chunk_size);
                self.feedback.process_block(&mut self.feedback_block, chunk_size);
                self.wet_mix.process_block(&mut self.wet_mix_block, chunk_size);
                self.filter_freq.process_block(&mut self.filter_freq_block, chunk_size);
                self.filter_resonance.process_block(&mut self.filter_resonance_block, chunk_size);

                self.block_position = 0;
                self.current_block_size = chunk_size;
            }

            // Process single frame with pre-computed block values
            let frame_start = frame_idx * 2;
            let frame = &mut output[frame_start..frame_start + 2];

            let left_input = frame[0];
            let right_input = frame[1];

            // Read from block buffers (fast array access)
            let delay_ms = self.delay_block[self.block_position];
            let depth = self.depth_block[self.block_position];
            let feedback = self.feedback_block[self.block_position].clamp(-0.999, 0.999);
            let wet_mix = self.wet_mix_block[self.block_position];
            let wet_amount = wet_mix;
            let dry_amount = 1.0 - wet_mix;

            // Update LFOs if rate/phase changed (check once per block)
            if self.block_position == 0 && (self.rate.value_need_ramp() || self.phase.value_need_ramp()) {
                self.update_lfos();
            }

            // Filter the inputs
            let (filtered_left, filtered_right) = {
                let cutoff = self.filter_freq_block[self.block_position];
                let q = self.filter_resonance_block[self.block_position] + 0.707;

                // Update filter coefficients if parameters changed
                if self.block_position == 0 && (self.filter_freq.value_need_ramp() || self.filter_resonance.value_need_ramp()) {
                    self.filter_coefficients
                        .set(
                            self.filter_type.value().into(),
                            self.sample_rate,
                            cutoff,
                            q,
                            0.0,
                        )
                        .expect("Failed to set chorus filter parameters");
                }

                let filtered_left = self
                    .filter_left
                    .process_sample(&self.filter_coefficients, left_input as f64);
                let filtered_right = self
                    .filter_right
                    .process_sample(&self.filter_coefficients, right_input as f64);
                (filtered_left, filtered_right)
            };

            // Run the LFOs
            let delay_in_samples = delay_ms * self.sample_rate as f32 * 0.001;
            let depth_in_samples = self.lfo_range * depth;

            let left_lfo = self.left_osc.run();
            let right_lfo = self.right_osc.run();

            let left_delay_pos = 2.0 + delay_in_samples + (1.0 + left_lfo) * depth_in_samples;
            let right_delay_pos = 2.0 + delay_in_samples + (1.0 + right_lfo) * depth_in_samples;

            // Feed the delays
            let left_output =
                self.delay_buffer_left
                    .process([filtered_left as f32], feedback, left_delay_pos)[0];
            let right_output =
                self.delay_buffer_right
                    .process([filtered_right as f32], feedback, right_delay_pos)[0];

            // Calc the Output
            let out_l = left_input * dry_amount + left_output * wet_amount;
            let out_r = right_input * dry_amount + right_output * wet_amount;

            frame[0] = out_l;
            frame[1] = out_r;

            self.block_position += 1;
            frame_idx += 1;
        }

        // Move our LFO offset to keep our oscillators updated when changing the rate or phase
        let phase_inc = 2.0 * PI * self.rate.current_value() as f64 / self.sample_rate as f64;
        self.current_phase += frame_count as f64 * phase_inc;
        while self.current_phase >= 2.0 * PI {
            self.current_phase -= 2.0 * PI;
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // Delay lines with feedback: tail depends on actual delay + modulation depth,
        // multiplied by feedback decay factor
        let delay_ms = self.delay.target_value();
        let depth_ms = Self::MAX_APPLIED_RANGE_IN_SAMPLES * 1000.0 / self.sample_rate as f32;
        let total_delay_ms = delay_ms + depth_ms;
        let feedback = self.feedback.target_value().abs();
        if feedback >= 1.0 {
            Some(usize::MAX) // tail is infinite
        } else if feedback < 0.001 {
            // No significant feedback, just the delay time
            Some((total_delay_ms * self.sample_rate as f32 / 1000.0).ceil() as usize)
        } else {
            // Calculate decay time based on feedback
            const SILENCE: f64 = 0.001; // -60dB threshold
            let total_delay_samples = total_delay_ms * self.sample_rate as f32 / 1000.0;
            let decay_time_samples = total_delay_samples
                + (total_delay_samples as f64 * SILENCE.log10() / (feedback as f64).log10()) as f32;
            Some(decay_time_samples.ceil() as usize)
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
            _ if id == Self::RATE.id() => self.rate.apply_update(value),
            _ if id == Self::PHASE.id() => self.phase.apply_update(value),
            _ if id == Self::DEPTH.id() => self.depth.apply_update(value),
            _ if id == Self::FEEDBACK.id() => self.feedback.apply_update(value),
            _ if id == Self::DELAY.id() => self.delay.apply_update(value),
            _ if id == Self::WET_MIX.id() => self.wet_mix.apply_update(value),
            _ if id == Self::FILTER_TYPE.id() => self.filter_type.apply_update(value),
            _ if id == Self::FILTER_FREQ.id() => self.filter_freq.apply_update(value),
            _ if id == Self::FILTER_Q.id() => self.filter_resonance.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };
        match id {
            _ if id == Self::FILTER_TYPE.id() => self
                .filter_coefficients
                .set_filter_type(self.filter_type.value().into()),
            _ => Ok(()),
        }
    }
}
