use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    utils::{
        filter::svf::{SvfFilter, SvfFilterType},
        InterleavedBufferMut,
    },
    Error,
};

use std::any::Any;
use std::f64::consts::PI;

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
    /// Set all chorus parameters at once.
    Init(
        f32, // Rate
        f32, // Depth
        f32, // Feedback
        f32, // Delay
        f32, // Wet
        f32, // Phase
    ),
    /// Set LFO rate in Hz. Range: 0.01 to 10.0.
    SetRate(f32),
    /// Set LFO depth. Range: 0.0 to 1.0.
    SetDepth(f32),
    /// Set feedback amount. Range: -1.0 to 1.0.
    SetFeedback(f32),
    /// Set base delay time in milliseconds. Range: 0.0 to 100.0.
    SetDelay(f32),
    /// Set wet/dry mix. Range: 0.0 (dry) to 1.0 (wet).
    SetWet(f32),
    /// Set phase offset between left and right LFOs in radians. Range: 0.0 to PI.
    SetPhase(f32),
    /// Reset LFO phase and delay lines.
    Reset,
    /// Set the filter type for the feedback path.
    SetFilterType(ChorusFilterType),
    /// Set filter cutoff frequency in Hz.
    SetFilterFreq(f32),
    /// Set filter resonance (Q factor). Range: 0.0 to 1.0.
    SetFilterResonance(f32),
}

impl EffectMessage for ChorusEffectMessage {
    fn effect_name(&self) -> &'static str {
        ChorusEffect::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

pub type ChorusFilterType = SvfFilterType;

// -------------------------------------------------------------------------------------------------

/// A stereo chorus effect with an filtered, interpolated delay-line.
pub struct ChorusEffect {
    sample_rate: u32,
    channel_count: usize,

    // Parameters
    rate: f64,
    phase: f64,
    depth: f64,
    feedback: f64,
    delay: f64,
    wet_mix: f64,
    filter_type: ChorusFilterType,
    filter_freq: f32,
    filter_resonance: f32,

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
    const MAX_APPLIED_RANGE_IN_SAMPLES: f64 = 256.0;
    const MAX_APPLIED_DELAY_IN_MS: f64 = 100.0;

    const DEFAULT_RATE: f64 = 1.0;
    const DEFAULT_PHASE: f64 = PI / 2.0;
    const DEFAULT_DEPTH: f64 = 0.25;
    const DEFAULT_FEEDBACK: f64 = 0.5;
    const DEFAULT_DELAY: f64 = 12.0;
    const DEFAULT_WET_MIX: f64 = 0.5;
    const DEFAULT_FILTER_TYPE: ChorusFilterType = ChorusFilterType::Highpass;
    const DEFAULT_FILTER_FREQ: f32 = 400.0;
    const DEFAULT_FILTER_RESONANCE: f32 = 0.3;

    #[allow(clippy::too_many_arguments)]
    pub fn with_parameters(
        rate: f64,
        phase: f64,
        depth: f64,
        feedback: f64,
        delay: f64,
        wet_mix: f64,
        filter_type: ChorusFilterType,
        filter_freq: f32,
        filter_resonance: f32,
    ) -> Self {
        Self {
            sample_rate: 0,
            channel_count: 0,

            rate: rate.clamp(0.01, 10.0),
            phase: phase.clamp(0.0, PI),
            depth: depth.clamp(0.0, 1.0),
            feedback: feedback.clamp(-1.0, 1.0),
            delay: delay.clamp(0.0, Self::MAX_APPLIED_DELAY_IN_MS),
            wet_mix: wet_mix.clamp(0.0, 1.0),
            filter_type,
            filter_freq: filter_freq.clamp(20.0, 22050.0),
            filter_resonance: filter_resonance.clamp(0.0, 1.0),

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

    fn update_lfos(&mut self, offset: Option<f64>) {
        if let Some(off) = offset {
            self.current_phase = off * 2.0 * PI;
        }
        self.left_osc.set_rate(self.rate, self.sample_rate);
        self.right_osc.set_rate(self.rate, self.sample_rate);
        self.left_osc.set_phase(self.current_phase);
        self.right_osc.set_phase(self.current_phase + self.phase);
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
        Self::with_parameters(
            Self::DEFAULT_RATE,
            Self::DEFAULT_PHASE,
            Self::DEFAULT_DEPTH,
            Self::DEFAULT_FEEDBACK,
            Self::DEFAULT_DELAY,
            Self::DEFAULT_WET_MIX,
            Self::DEFAULT_FILTER_TYPE,
            Self::DEFAULT_FILTER_FREQ,
            Self::DEFAULT_FILTER_RESONANCE,
        )
    }
}

impl Effect for ChorusEffect {
    fn name() -> &'static str {
        "ChorusEffect"
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
            self.filter_type,
            sample_rate,
            self.filter_freq,
            self.filter_resonance + 0.707,
            1.0,
        )?;
        self.filter_bank_right = SvfFilter::new(
            self.filter_type,
            sample_rate,
            self.filter_freq,
            self.filter_resonance + 0.707,
            1.0,
        )?;

        self.reset();

        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        let delay_ms = self.delay;
        let depth = self.depth;
        let feedback = self.feedback.clamp(-0.999, 0.999);
        let wet_amount = self.wet_mix;
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
        let phase_inc = 2.0 * PI * self.rate / self.sample_rate as f64;
        self.current_phase += output.len() as f64 / self.channel_count as f64 * phase_inc;
        while self.current_phase >= 2.0 * PI {
            self.current_phase -= 2.0 * PI;
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<ChorusEffectMessage>() {
            match message {
                ChorusEffectMessage::Init(rate, depth, feedback, delay, wet, phase) => {
                    self.rate = *rate as f64;
                    self.depth = *depth as f64;
                    self.feedback = *feedback as f64;
                    self.delay = (*delay as f64).clamp(0.0, Self::MAX_APPLIED_DELAY_IN_MS);
                    self.wet_mix = *wet as f64;
                    self.phase = *phase as f64;
                    self.update_lfos(None);
                }
                ChorusEffectMessage::SetRate(rate) => {
                    self.rate = *rate as f64;
                    self.update_lfos(None);
                }
                ChorusEffectMessage::SetDepth(depth) => self.depth = *depth as f64,
                ChorusEffectMessage::SetFeedback(feedback) => self.feedback = *feedback as f64,
                ChorusEffectMessage::SetDelay(delay) => {
                    self.delay = (*delay as f64).clamp(0.0, Self::MAX_APPLIED_DELAY_IN_MS)
                }
                ChorusEffectMessage::SetWet(wet) => self.wet_mix = *wet as f64,
                ChorusEffectMessage::SetPhase(phase) => {
                    self.phase = *phase as f64;
                    self.update_lfos(None);
                }
                ChorusEffectMessage::Reset => self.reset(),
                ChorusEffectMessage::SetFilterType(ft) => {
                    self.filter_type = *ft;
                    let _ = self
                        .filter_bank_left
                        .coefficients_mut()
                        .set_filter_type(*ft);
                    let _ = self
                        .filter_bank_right
                        .coefficients_mut()
                        .set_filter_type(*ft);
                }
                ChorusEffectMessage::SetFilterFreq(c) => {
                    self.filter_freq = *c;
                    let _ = self.filter_bank_left.coefficients_mut().set_cutoff(*c);
                    let _ = self.filter_bank_right.coefficients_mut().set_cutoff(*c);
                }
                ChorusEffectMessage::SetFilterResonance(q) => {
                    self.filter_resonance = *q;
                    let _ = self.filter_bank_left.coefficients_mut().set_q(*q + 0.707);
                    let _ = self.filter_bank_right.coefficients_mut().set_q(*q + 0.707);
                }
            }
        } else {
            log::error!("ChorusEffect: Invalid/unknown message payload");
        }
    }
}
