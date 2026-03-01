use four_cc::FourCC;
use strum::{EnumCount, VariantNames};

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        formatters, EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        buffer::InterleavedBufferMut,
        smoothing::{ExponentialSmoothedValue, LinearSmoothedValue},
    },
    Error, Parameter,
};

// -------------------------------------------------------------------------------------------------

/// Type of distortion applied in `DistortionEffect`.
#[derive(
    Debug,
    Default,
    Copy,
    Clone,
    PartialEq,
    strum::Display,
    strum::EnumString,
    strum::VariantNames,
    strum::VariantArray,
    strum::EnumCount,
)]
pub enum DistortionType {
    /// Soft clipping distortion using a cubic polynomial.
    ///
    /// Creates a warm, smooth saturation by gently rounding off signal peaks, similar to
    /// overdriving analog tape or a vacuum tube.
    #[default]
    SoftClip,

    /// Classic hard clipping distortion.
    ///
    /// Produces a buzzy, aggressive distortion by abruptly chopping off any part of the
    /// signal that exceeds a sharp threshold, like a transistor-based fuzz pedal.
    HardClip,

    /// Exponential shockley diode model followed by an atan soft-clipper.
    ///
    /// Simulates the asymmetric clipping of a semiconductor diode, yielding a bright and
    /// harmonically rich distortion characteristic of many classic overdrive pedals.
    Diode,

    /// Symmetrical saturation followed by half-wave rectification.
    ///
    /// Generates a gritty, vintage fuzz tone by amplifying the signal and then removing
    /// the entire negative half of the waveform.
    Fuzz,

    /// Wavefolder distortion that reflects the signal back at a threshold.
    ///
    /// Produces a metallic, bell-like timbre by folding peaks back into the waveform
    /// rather than clipping them, generating complex upper harmonics with a distinctly
    /// electronic character.
    Fold,
}

// -------------------------------------------------------------------------------------------------

impl DistortionType {
    // const MIN_DRIVE: f32 = 0.0;
    const MAX_DRIVE: f32 = 4.0;

    /// Get waveshaper function for the distortion mode.
    pub(self) fn shape_function(&self) -> fn(f32, f32) -> f32 {
        match self {
            DistortionType::SoftClip => Self::soft_clip,
            DistortionType::HardClip => Self::hard_clip,
            DistortionType::Diode => Self::diode,
            DistortionType::Fuzz => Self::fuzz,
            DistortionType::Fold => Self::fold,
        }
    }

    /// Dynamically compute RMS-based gain compensation for a waveshaper function.
    ///
    /// Evaluates the shaper over one period of a composite signal, a sum of inharmonic partials
    /// with decreasing amplitudes and returns the ratio `input_rms / output_rms`, so that the processed
    /// signal roughly matches the perceived loudness of the dry signal at the given drive level.
    pub(self) fn rms_compensation(&self, drive: f32) -> f32 {
        use std::f32::consts;

        const N: usize = 256;
        const PARTIALS: [(f32, f32); 5] = [
            (1.0, 0.60),
            (2.7, 0.25),
            (5.3, 0.10),
            (9.1, 0.03),
            (14.6, 0.02),
        ];
        let partials_peak = PARTIALS.iter().map(|(_, v)| v).sum::<f32>();

        let shaper = self.shape_function();

        let mut input_sum_sq = 0.0f32;
        let mut output_sum_sq = 0.0f32;
        for i in 0..N {
            let t = consts::TAU * (i as f32 + 0.5) / N as f32;
            let sample: f32 = PARTIALS
                .iter()
                .map(|(freq, amp)| amp * (freq * t).sin())
                .sum::<f32>()
                / partials_peak;
            input_sum_sq += sample * sample;
            output_sum_sq += shaper(sample, drive).powi(2);
        }
        let input_rms = (input_sum_sq / N as f32).sqrt();
        let output_rms = (output_sum_sq / N as f32).sqrt();
        if output_rms > 1e-10 {
            input_rms / output_rms
        } else {
            1.0
        }
    }

    #[inline]
    fn soft_clip(sample: f32, drive: f32) -> f32 {
        const BOOST: f32 = 15.0;
        let t = drive / Self::MAX_DRIVE;
        let gain = 1.0 + t.powi(2) * (BOOST - 1.0);
        let x = sample * gain;
        if x >= 1.0 {
            1.0
        } else if x > -1.0 {
            if gain <= 1.0 {
                sample // passthrough — no clipping possible
            } else {
                (3.0 / 2.0) * (x - x.powi(3) / 3.0)
            }
        } else {
            -1.0
        }
    }

    #[inline]
    fn hard_clip(sample: f32, drive: f32) -> f32 {
        const BOOST: f32 = 25.0;
        let t = drive / Self::MAX_DRIVE;
        let gain = 1.0 + t.powi(2) * (BOOST - 1.0);
        let threshold = 1.0 / gain;
        sample.clamp(-threshold, threshold) * gain
    }

    #[inline]
    fn diode(sample: f32, drive: f32) -> f32 {
        const BOOST: f32 = 20.0;
        let t = drive / Self::MAX_DRIVE;
        let curve = 0.6 * t.powi(2) + 0.4 * t;
        let gain = 1.0 + curve * (BOOST - 1.0);
        let diode_clipping = ((0.1 * sample) / (0.0253 * 1.68)).exp() - 1.0;
        2.0 / std::f32::consts::PI * (diode_clipping * gain).atan()
    }

    #[inline]
    fn fuzz(sample: f32, drive: f32) -> f32 {
        const BOOST: f32 = 30.0;
        let t = drive / Self::MAX_DRIVE;
        let gain = 1.0 + (1.0 - (-3.0 * t).exp()) * (BOOST - 1.0);
        let amplified = sample * gain;
        #[allow(clippy::neg_multiply)]
        let saturated = if amplified < 0.0 {
            -1.0 * (1.0 - (-amplified.abs()).exp())
        } else {
            1.0 * (1.0 - (-amplified.abs()).exp())
        };
        1.5 * (saturated + saturated.abs())
    }

    #[inline]
    fn fold(sample: f32, drive: f32) -> f32 {
        const BOOST: f32 = 4.0;
        let t = drive / Self::MAX_DRIVE;
        let gain = 1.0 + t.powi(2) * (BOOST - 1.0);
        let x = sample * gain;
        let threshold = 1.0 / gain;
        if x > threshold || x < -threshold {
            ((x - threshold).abs() % (threshold * 4.0) - threshold * 2.0).abs() - threshold
        } else {
            x
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Multi-channel distortion effect with multiple waveshaping algorithms.
#[derive(Debug)]
pub struct DistortionEffect {
    // Parameters
    distortion_type: EnumParameterValue<DistortionType>,
    drive: SmoothedParameterValue<LinearSmoothedValue>,
    mix: SmoothedParameterValue<ExponentialSmoothedValue>,
    // Internal State
    compensation_luts: [[f32; Self::GAIN_COMPENSATION_LUT_SIZE]; DistortionType::COUNT],
    channel_count: usize,
}

impl DistortionEffect {
    pub const EFFECT_NAME: &str = "Distortion";

    pub const TYPE: EnumParameter = EnumParameter::new(
        FourCC(*b"type"),
        "Type",
        DistortionType::VARIANTS,
        DistortionType::Diode as usize,
    );
    pub const DRIVE: FloatParameter = FloatParameter::new(
        FourCC(*b"driv"),
        "Drive",
        0.0..=DistortionType::MAX_DRIVE,
        0.0, //
    )
    .with_unit("x");
    pub const MIX: FloatParameter = FloatParameter::new(
        FourCC(*b"mix "),
        "Mix",
        0.0..=1.0,
        1.0, //
    )
    .with_formatter(formatters::PERCENT);

    const GAIN_COMPENSATION_LUT_SIZE: usize = 256;

    /// Creates a new `DistortionEffect` with default parameter values.
    pub fn new() -> Self {
        let channel_count = 0;

        let distortion_type = EnumParameterValue::from_description(Self::TYPE);

        let drive = SmoothedParameterValue::from_description(Self::DRIVE)
            .with_smoother(LinearSmoothedValue::default().with_step(0.01));

        let mix = SmoothedParameterValue::from_description(Self::MIX)
            .with_smoother(ExponentialSmoothedValue::default().with_inertia(0.1));

        let compensation_luts = Self::build_gain_compensation_table();

        Self {
            channel_count,
            distortion_type,
            drive,
            mix,
            compensation_luts,
        }
    }

    /// Creates a new `DistortionEffect` with the given parameters.
    pub fn with_parameters(distortion_type: DistortionType, drive: f32, mix: f32) -> Self {
        let mut distortion = Self::default();
        distortion.distortion_type.set_value(distortion_type);
        distortion.drive.init_value(drive);
        distortion.mix.init_value(mix);
        distortion
    }

    /// Precompute RMS compensation lookup tables for all distortion types.
    fn build_gain_compensation_table(
    ) -> [[f32; Self::GAIN_COMPENSATION_LUT_SIZE]; DistortionType::COUNT] {
        let mut luts = [[0.0f32; Self::GAIN_COMPENSATION_LUT_SIZE]; DistortionType::COUNT];
        for &shape in <DistortionType as strum::VariantArray>::VARIANTS {
            let lut_index = shape as usize;
            for (i, entry) in luts[lut_index].iter_mut().enumerate() {
                let drive = i as f32 / (Self::GAIN_COMPENSATION_LUT_SIZE - 1) as f32
                    * DistortionType::MAX_DRIVE;
                *entry = shape.rms_compensation(drive);
            }
        }
        luts
    }

    /// Look up the RMS compensation for the given LUT index and drive value.
    fn lookup_gain_compensation(&self, lut_index: usize, drive: f32) -> f32 {
        let lut = &self.compensation_luts[lut_index];
        let pos = (drive / DistortionType::MAX_DRIVE).clamp(0.0, 1.0)
            * (Self::GAIN_COMPENSATION_LUT_SIZE - 1) as f32;
        let lo = pos as usize;
        let hi = (lo + 1).min(Self::GAIN_COMPENSATION_LUT_SIZE - 1);
        let frac = pos - lo as f32;
        lut[lo] + (lut[hi] - lut[lo]) * frac
    }
}

impl Default for DistortionEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for DistortionEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        1
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![
            self.distortion_type.description(),
            self.drive.description(),
            self.mix.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.channel_count = channel_count;
        self.mix.set_sample_rate(sample_rate);
        self.drive.set_sample_rate(sample_rate);
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        let shape_function = self.distortion_type.value().shape_function();
        let lut_index = self.distortion_type.value() as usize;

        // process, avoid mixing and ramping if not needed...
        if !self.mix.value_need_ramp() && self.mix.target_value() == 0.0 {
            // nothing to do
        } else if !self.mix.value_need_ramp() && self.mix.target_value() >= 1.0 {
            if !self.drive.value_need_ramp() {
                let drive = self.drive.target_value();
                let compensation = self.lookup_gain_compensation(lut_index, drive);
                for sample in output.iter_mut() {
                    *sample = shape_function(*sample, drive) * compensation;
                }
            } else {
                for frame in output.frames_mut(self.channel_count) {
                    let drive = self.drive.next_value();
                    let compensation = self.lookup_gain_compensation(lut_index, drive);
                    for sample in frame {
                        *sample = shape_function(*sample, drive) * compensation;
                    }
                }
            }
        } else {
            for frame in output.frames_mut(self.channel_count) {
                let drive = self.drive.next_value();
                let compensation = self.lookup_gain_compensation(lut_index, drive);
                let mix = self.mix.next_value();
                for sample in frame {
                    let dry = *sample;
                    let wet = shape_function(dry, drive) * compensation;
                    *sample = (1.0 - mix) * dry + mix * wet;
                }
            }
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // Memoryless waveshaping with no internal state - no tail
        Some(0)
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::TYPE.id() => self.distortion_type.apply_update(value),
            _ if id == Self::DRIVE.id() => self.drive.apply_update(value),
            _ if id == Self::MIX.id() => self.mix.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        }
        Ok(())
    }
}
