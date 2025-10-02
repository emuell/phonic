use std::any::Any;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    Error,
};

// -------------------------------------------------------------------------------------------------

/// The type of distortion to apply.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
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

/// Message type for `DistortionEffect` to change parameters.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum DistortionEffectMessage {
    /// Set the distortion type.
    SetType(DistortionType),
    /// Set the drive amount. Range: 0.0 to 1.0.
    SetDrive(f32),
    /// Set the wet/dry mix. Range: 0.0 (dry) to 1.0 (wet).
    SetMix(f32),
}

impl EffectMessage for DistortionEffectMessage {
    fn effect_name(&self) -> &'static str {
        DistortionEffect::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A simple distortion effect with multiple waveshaping algorithms.
#[derive(Debug)]
pub struct DistortionEffect {
    distortion_type: DistortionType,
    drive: f32,
    mix: f32,
}

impl DistortionEffect {
    const DEFAULT_TYPE: DistortionType = DistortionType::Diode;
    const DEFAULT_DRIVE: f32 = 0.5;
    const DEFAULT_MIX: f32 = 1.0;

    /// Creates a new `DistortionEffect` with the given parameters.
    pub fn with_parameters(distortion_type: DistortionType, drive: f32, mix: f32) -> Self {
        Self {
            distortion_type,
            drive: drive.clamp(0.0, 2.0),
            mix: mix.clamp(0.0, 1.0),
        }
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
        let dist_function = match self.distortion_type {
            DistortionType::None => return,
            DistortionType::SoftClip => Self::soft_clip,
            DistortionType::HardClip => Self::hard_clip,
            DistortionType::Diode => Self::diode,
            DistortionType::Fuzz => Self::fuzz,
        };
        if self.mix >= 1.0 {
            for sample in output {
                *sample = dist_function(*sample, self.drive);
            }
        } else {
            for sample in output {
                let dry = *sample;
                let wet = dist_function(dry, self.drive);
                *sample = (1.0 - self.mix) * dry + self.mix * wet;
            }
        }
    }
}

impl Default for DistortionEffect {
    fn default() -> Self {
        Self {
            distortion_type: Self::DEFAULT_TYPE,
            drive: Self::DEFAULT_DRIVE,
            mix: Self::DEFAULT_MIX,
        }
    }
}

impl Effect for DistortionEffect {
    fn name() -> &'static str {
        "DistortionEffect"
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
        if self.distortion_type == DistortionType::None || self.mix == 0.0 {
            return;
        }
        self.process(output.iter_mut());
    }

    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<DistortionEffectMessage>() {
            match message {
                DistortionEffectMessage::SetType(t) => self.distortion_type = *t,
                DistortionEffectMessage::SetDrive(d) => self.drive = d.clamp(0.0, 10.0),
                DistortionEffectMessage::SetMix(m) => self.mix = m.clamp(0.0, 1.0),
            }
        } else {
            log::error!("DistortionEffect: Invalid/unknown message payload");
        }
    }
}
