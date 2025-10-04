use four_cc::FourCC;
use strum::{Display, EnumIter, EnumString};

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, FloatParameterValue, Parameter,
        ParameterValueUpdate,
    },
    Error,
};

// -------------------------------------------------------------------------------------------------

/// The type of distortion to apply.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default, Display, EnumIter, EnumString)]
pub enum DistortionType {
    /// No distortion.
    #[default]
    None,
    /// Soft clipping distortion using a cubic polynomial.
    ///
    /// Creates a warm, smooth saturation by gently rounding off signal peaks, similar to
    /// overdriving analog tape or a vacuum tube.
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
}

// -------------------------------------------------------------------------------------------------

/// A simple distortion effect with multiple waveshaping algorithms.
#[derive(Debug)]
pub struct DistortionEffect {
    distortion_type: EnumParameterValue<DistortionType>,
    drive: FloatParameterValue,
    mix: FloatParameterValue,
}

impl DistortionEffect {
    pub const EFFECT_NAME: &str = "DistortionEffect";
    pub const TYPE_ID: FourCC = FourCC(*b"type");
    pub const DRIVE_ID: FourCC = FourCC(*b"driv");
    pub const MIX_ID: FourCC = FourCC(*b"mix ");

    /// Creates a new `DistortionEffect` with default parameter values.
    pub fn new() -> Self {
        Self {
            distortion_type: EnumParameter::new(Self::TYPE_ID, "Type", DistortionType::Diode)
                .into(),
            drive: FloatParameter::new(Self::DRIVE_ID, "Drive", 0.0..=2.0, 0.5).into(),
            mix: FloatParameter::new(Self::MIX_ID, "Mix", 0.0..=1.0, 1.0).into(),
        }
    }

    /// Creates a new `DistortionEffect` with the given parameters.
    pub fn with_parameters(distortion_type: DistortionType, drive: f32, mix: f32) -> Self {
        let mut distortion = Self::default();
        distortion.distortion_type.set_value(distortion_type);
        distortion.drive.set_value(drive);
        distortion.mix.set_value(mix);
        distortion
    }

    #[inline]
    fn soft_clip(sample: f32, drive: f32) -> f32 {
        const BOOST_FACTOR: f32 = 15.0;
        let gain = 1.0 + drive.powi(4) * (BOOST_FACTOR - 1.0);
        let gain_compensation = 1.0 - (drive.min(1.0) / 2.0).powi(2) * 2.0;
        let amplified_sample = sample * gain;
        if amplified_sample >= 1.0 {
            gain_compensation
        } else if amplified_sample > -1.0 {
            (3.0 / 2.0) * (amplified_sample - amplified_sample.powi(3) / 3.0) * gain_compensation
        } else {
            -gain_compensation
        }
    }

    #[inline]
    fn hard_clip(sample: f32, drive: f32) -> f32 {
        const BOOST_FACTOR: f32 = 50.0;
        let gain = 1.0 + drive.powi(4) * (BOOST_FACTOR - 1.0);
        let gain_compensation = 1.0 - (drive.min(1.0) / 2.0).powi(2) * 2.0;
        let threshold = 1.0 / gain;
        let clamped_sample = sample.clamp(-threshold, threshold);
        clamped_sample * gain * gain_compensation
    }

    #[inline]
    fn diode(sample: f32, drive: f32) -> f32 {
        const BOOST_FACTOR: f32 = 20.0;
        let gain = 1.0 + drive.powi(4) * (BOOST_FACTOR - 1.0);
        let gain_compensation = 1.0 - (drive.min(1.0) / 2.0).powi(2) * 2.0;
        let diode_clipping = ((0.1 * sample) / (0.0253 * 1.68)).exp() - 1.0;
        2.0 / std::f32::consts::PI * (diode_clipping * gain).atan() * gain_compensation
    }

    #[inline]
    fn fuzz(sample: f32, drive: f32) -> f32 {
        const BOOST_FACTOR: f32 = 30.0;
        let gain = 1.0 + drive.powi(4) * (BOOST_FACTOR - 1.0);
        let gain_compensation = 1.0 - (drive.min(1.0) / 2.0).powi(2) * 2.0;
        let amplified_sample = sample * gain;
        #[allow(clippy::neg_multiply)]
        let saturated = if amplified_sample < 0.0 {
            -1.0 * (1.0 - (-amplified_sample.abs()).exp())
        } else {
            1.0 * (1.0 - (-amplified_sample.abs()).exp())
        };
        1.5 * (saturated + saturated.abs()) * gain_compensation
    }

    /// Process helper function that calls `process_sample` for each sample in a buffer
    #[inline]
    pub fn process<'a>(&mut self, output: impl Iterator<Item = &'a mut f32>) {
        let dist_function = match *self.distortion_type.value() {
            DistortionType::None => return,
            DistortionType::SoftClip => Self::soft_clip,
            DistortionType::HardClip => Self::hard_clip,
            DistortionType::Diode => Self::diode,
            DistortionType::Fuzz => Self::fuzz,
        };
        if *self.mix.value() >= 1.0 {
            for sample in output {
                *sample = dist_function(*sample, *self.drive.value());
            }
        } else {
            for sample in output {
                let dry = *sample;
                let wet = dist_function(dry, *self.drive.value());
                *sample = (1.0 - *self.mix.value()) * dry + *self.mix.value() * wet;
            }
        }
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

    fn parameters(&self) -> Vec<Box<dyn Parameter>> {
        vec![
            Box::new(self.distortion_type.description().clone()),
            Box::new(self.drive.description().clone()),
            Box::new(self.mix.description().clone()),
        ]
    }

    fn initialize(
        &mut self,
        _sample_rate: u32,
        _channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        Ok(())
    }

    fn process(&mut self, output: &mut [f32], _time: &EffectTime) {
        if *self.distortion_type.value() == DistortionType::None || *self.mix.value() == 0.0 {
            return;
        }
        self.process(output.iter_mut());
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            Self::TYPE_ID => self.distortion_type.apply_update(value),
            Self::DRIVE_ID => self.drive.apply_update(value),
            Self::MIX_ID => self.mix.apply_update(value),
            _ => return Err(Error::ParameterError(format!("Unknown parameter: {id}"))),
        }
        Ok(())
    }
}
