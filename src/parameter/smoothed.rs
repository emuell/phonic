use std::fmt::{Debug, Display};

use crate::utils::smoothing::{ExponentialSmoothedValue, SmoothedValue};

use super::{FloatParameter, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// Holds a float parameter value and its description, using a [`SmoothedValue`] instance to
/// smoothly update the value on changes.
///
/// The smoothed value needs a valid sample rate set. So make sure to call [`Self::set_sample_rate`]
/// as soon as the parameter's effect gets initialized.
///
/// To configure step sizes or inertia of the smoother, use [`Self::with_smoother`].
#[derive(Debug, Clone)]
pub struct SmoothedParameterValue<Value: SmoothedValue = ExponentialSmoothedValue> {
    /// The parameter's description and constraints.
    description: FloatParameter,
    /// The smoothed value of the parameter.
    value: Value,
}

impl<Value: SmoothedValue> SmoothedParameterValue<Value> {
    /// Create a new SmoothedParameterValue with the given parameter, using
    /// a default instance of a smoother, initialized to the parameter's default value.
    ///
    /// NB: Call `set_sample_rate` before using the parameter value to property set up
    /// the default constructed smoother!
    pub fn from_description(description: FloatParameter) -> Self
    where
        Value: From<f32>,
    {
        let value = Value::from(description.default_value());
        Self { value, description }
    }

    /// Create a smoothed value with the given smoother instance. The instance's value
    /// will be set to the parameter's default value - all other smoother properties
    /// are kept intact.
    pub fn with_smoother(mut self, value: Value) -> Self {
        self.value = value;
        self.value.init(self.description.default_value());
        self
    }

    /// Access the parameter value's description.
    pub fn description(&self) -> &FloatParameter {
        &self.description
    }

    /// Set a sample rate for the smoother. Must be called before using the value!
    pub fn set_sample_rate(&mut self, sample_rate: u32) {
        self.value.set_sample_rate(sample_rate)
    }

    /// Test if ramping is necessary. When not, `target_value` can be used directly without
    /// ramping to avoid processing overhead.
    pub fn value_need_ramp(&self) -> bool {
        self.value.need_ramp()
    }

    /// Apply smoothing, if needed, and return current value. This should be called once
    /// per sample frame in effects.
    #[inline(always)]
    pub fn next_value(&mut self) -> f32 {
        self.value.next()
    }

    /// Access to the smoothed current value.
    #[inline(always)]
    pub fn current_value(&self) -> f32 {
        self.value.current()
    }

    /// Access to the smoothed target value.
    #[inline(always)]
    pub fn target_value(&self) -> f32 {
        self.value.target()
    }

    /// Set a new smoothed target value.
    pub fn set_target_value(&mut self, value: f32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value.set_target(value);
    }

    /// Set a new smoothed target value, clamping the given value into the
    /// parameter's value bounds if necessary.
    pub fn set_target_value_clamped(&mut self, value: f32) {
        self.value.set_target(self.description.clamp_value(value));
    }

    /// Initialize the smoothed value so that no smoothing is performed.
    pub fn init_value(&mut self, value: f32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value.init(value);
    }

    /// Initialize the smoothed value so that no smoothing is performed, clamping the
    /// given value into the parameter's value bounds if necessary.
    pub fn init_value_clamped(&mut self, value: f32) {
        self.value.init(self.description.clamp_value(value));
    }

    /// Applies a parameter update by setting a new target value.
    /// To disable smoothing call `param.init_value(param.target_value())` afterwards.
    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<f32>() {
                    self.set_target_value_clamped(*value);
                } else if let Some(value) = raw.downcast_ref::<f64>() {
                    self.set_target_value_clamped(*value as f32);
                } else {
                    log::warn!(
                        "Invalid value type for float parameter '{}'",
                        self.description.id()
                    );
                }
            }
            ParameterValueUpdate::Normalized(normalized) => {
                let value = self
                    .description
                    .denormalize_value(normalized.clamp(0.0, 1.0));
                self.set_target_value(value);
            }
        }
    }
}

impl<Value: SmoothedValue> From<FloatParameter> for SmoothedParameterValue<Value>
where
    Value: From<f32>,
{
    fn from(description: FloatParameter) -> Self {
        Self::from_description(description)
    }
}

impl<Value: SmoothedValue> Display for SmoothedParameterValue<Value> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let include_unit = true;
        f.write_str(
            &self
                .description
                .value_to_string(self.value.target(), include_unit),
        )
    }
}
