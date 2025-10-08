use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{FloatParameter, ParameterValueUpdate, SmoothedParameterValue},
    utils::{buffer::scale_buffer, db_to_linear, linear_to_db, InterleavedBufferMut},
    ClonableParameter, Error, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// A simple gain effect that only applies a volume factor.
pub struct GainEffect {
    channel_count: usize,
    gain: SmoothedParameterValue,
}

impl GainEffect {
    pub const EFFECT_NAME: &str = "GainEffect";
    pub const GAIN_ID: FourCC = FourCC(*b"gain");

    const MIN_DB: f32 = -60.0;
    const MAX_DB: f32 = 12.0;

    /// Creates a new `GainEffect` with default gain (0dB = unity gain).
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            gain: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::GAIN_ID,
                    "Gain",
                    db_to_linear(Self::MIN_DB)..=db_to_linear(Self::MAX_DB),
                    1.0, // 0dB = unity gain
                )
                .with_unit("dB")
                .with_scaling(ParameterScaling::Decibel(Self::MIN_DB, Self::MAX_DB))
                .with_display(
                    |v: f32| {
                        let db = linear_to_db(v);
                        if db <= -59.0 {
                            "-INF".to_string()
                        } else {
                            format!("{:.2}", db)
                        }
                    },
                    |s: &str| {
                        if s.trim().eq_ignore_ascii_case("-inf")
                            || s.trim().eq_ignore_ascii_case("inf")
                        {
                            Some(db_to_linear(Self::MIN_DB))
                        } else {
                            s.parse::<f32>().ok().map(db_to_linear)
                        }
                    },
                ),
            ),
        }
    }

    /// Creates a new `GainEffect` with the given gain in dB.
    pub fn with_gain_db(gain_db: f32) -> Self {
        let mut effect = Self::new();
        let gain_linear = db_to_linear(gain_db.clamp(Self::MIN_DB, Self::MAX_DB));
        effect.gain.init_value(gain_linear);
        effect
    }

    /// Get the current gain in dB.
    pub fn gain_db(&self) -> f32 {
        linear_to_db(self.gain.target_value())
    }

    /// Set the gain in dB.
    pub fn set_gain_db(&mut self, gain_db: f32) {
        let gain_linear = db_to_linear(gain_db.clamp(Self::MIN_DB, Self::MAX_DB));
        self.gain.set_target_value(gain_linear);
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

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
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

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            Self::GAIN_ID => {
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
