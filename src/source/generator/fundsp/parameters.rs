use crate::{parameters::FloatParameter, Parameter, ParameterValueUpdate};

use fundsp::hacker32::Shared;

// -------------------------------------------------------------------------------------------------

/// Holds a fundsp [Shared] float parameter value and its description as [FloatParameter].
///
/// Shared parameters can be added via `var(param.shared())` to fun DSP factory function in order
/// to dynamically apply parameter automation.
#[derive(Clone)]
pub struct FunDspFloatParameterValue {
    /// The parameter's description and constraints.
    description: FloatParameter,
    /// The current value of the parameter.
    value: Shared,
}

impl FunDspFloatParameterValue {
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: FloatParameter) -> Self {
        let value = Shared::new(description.default_value());
        Self { value, description }
    }

    /// Access the parameter value's description.
    pub fn description(&self) -> &FloatParameter {
        &self.description
    }

    /// Access to the shared value.
    #[inline(always)]
    pub fn shared(&self) -> &Shared {
        &self.value
    }

    /// Access to the current value.
    #[inline(always)]
    #[allow(unused)]
    pub fn value(&self) -> f32 {
        self.value.value()
    }

    /// Set a new value.
    pub fn set_value(&mut self, value: f32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value.set_value(value);
    }

    /// Set a new value, clamping the given value into the parameter's value bounds if necessary.
    pub fn set_value_clamped(&mut self, value: f32) {
        self.value.set_value(self.description.clamp_value(value));
    }

    /// Applies a parameter update.
    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = (*raw).downcast_ref::<f32>() {
                    self.set_value_clamped(*value);
                } else if let Some(value) = (*raw).downcast_ref::<f64>() {
                    self.set_value_clamped(*value as f32);
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
                self.set_value(value);
            }
        }
    }
}
