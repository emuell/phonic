use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{formatters, FloatParameter, ParameterValueUpdate, SmoothedParameterValue},
    utils::{
        buffer::{scale_buffer, InterleavedBufferMut},
        db_to_linear,
    },
    Error, Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// Multi-channel gain effect that only applies a volume factor.
pub struct GainEffect {
    channel_count: usize,
    gain: SmoothedParameterValue,
}

impl GainEffect {
    pub const EFFECT_NAME: &str = "Gain";

    const MIN_DB: f32 = -60.0;
    const MAX_DB: f32 = 24.0;

    pub const GAIN: FloatParameter = FloatParameter::new(
        FourCC(*b"gain"),
        "Gain",
        0.000001..=15.848932, // Self::MIN_DB..=Self::MAX_DB,
        1.0,                  // 0dB
    )
    .with_scaling(ParameterScaling::Decibel(Self::MIN_DB, Self::MAX_DB))
    .with_formatter(formatters::GAIN);

    /// Creates a new `GainEffect` with default gain (0dB = unity gain).
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            gain: SmoothedParameterValue::from_description(Self::GAIN),
        }
    }

    /// Creates a new `GainEffect` with the given gain in dB.
    pub fn with_gain_db(gain_db: f32) -> Self {
        let mut effect = Self::new();
        let gain_linear = db_to_linear(gain_db.clamp(Self::MIN_DB, Self::MAX_DB));
        effect.gain.init_value(gain_linear);
        effect
    }
}

impl Default for GainEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for GainEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        1
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![self.gain.description()]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.channel_count = channel_count;
        self.gain.set_sample_rate(sample_rate);
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        if self.gain.value_need_ramp() {
            for frame in output.frames_mut(self.channel_count) {
                let gain = self.gain.next_value();
                for sample in frame {
                    *sample *= gain;
                }
            }
        } else {
            let gain = self.gain.target_value();
            scale_buffer(output, gain);
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // Gain is instantaneous with no memory - no tail
        Some(0)
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::GAIN.id() => {
                self.gain.apply_update(value);
                Ok(())
            }
            _ => Err(Error::ParameterError(format!(
                "Unknown parameter: '{id}' for effect '{}'",
                self.name()
            ))),
        }
    }
}
