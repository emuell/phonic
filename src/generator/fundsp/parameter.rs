use std::{any::Any, ops::RangeInclusive};

use crate::{
    parameter::{BooleanParameter, EnumParameter, FloatParameter, IntegerParameter},
    Parameter, ParameterScaling, ParameterValueUpdate,
};

use fundsp::hacker32::Shared;

// -------------------------------------------------------------------------------------------------

/// Holds a fundsp [Shared] float parameter value and its description as float value.
///
/// Shared parameters can be added via `var(param.shared())` to fundsp factory function in order
/// to automate parameters in the voices.
pub struct SharedParameterValue {
    /// The parameter's description and constraints.
    description: Box<dyn Parameter>,
    /// The parameter's range represented as floating point range.
    range: RangeInclusive<f32>,
    /// The parameter's scaling.
    scaling: ParameterScaling,
    /// The current value of the parameter.
    value: Shared,
}

impl SharedParameterValue {
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: &dyn Parameter) -> Self {
        let description_any = description as &dyn Any;

        let range;
        let scaling;
        if let Some(float_param) = description_any.downcast_ref::<FloatParameter>() {
            range = float_param.range().clone();
            scaling = *float_param.scaling();
        } else if let Some(integer_param) = description_any.downcast_ref::<IntegerParameter>() {
            range = RangeInclusive::new(
                *integer_param.range().start() as f32,
                *integer_param.range().end() as f32,
            );
            scaling = ParameterScaling::Linear;
        } else if let Some(enum_param) = description_any.downcast_ref::<EnumParameter>() {
            range = RangeInclusive::new(0.0, (enum_param.values().len() - 1) as f32);
            scaling = ParameterScaling::Linear;
        } else if description_any.downcast_ref::<BooleanParameter>().is_some() {
            range = RangeInclusive::new(0.0, 1.0);
            scaling = ParameterScaling::Linear;
        } else {
            unreachable!("Unexpected parameter type")
        }

        let default_normalized = description.default_value();
        let default_scaled = scaling.scale(default_normalized);
        let default_value = range.start() + default_scaled * (range.end() - range.start());

        let value = Shared::new(default_value);
        let description = description.dyn_clone();

        Self {
            value,
            range,
            scaling,
            description,
        }
    }

    /// Access the parameter value's description.
    pub fn description(&self) -> &dyn Parameter {
        self.description.as_ref()
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
        assert!(self.range.contains(&value), "Value out of bounds");
        self.value.set_value(value);
    }

    /// Set a new value, clamping the given value into the parameter's value bounds if necessary.
    pub fn set_value_clamped(&mut self, value: f32) {
        self.value
            .set_value(value.clamp(*self.range.start(), *self.range.end()));
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
                let normalized = normalized.clamp(0.0, 1.0);
                let normalized_scaled = self.scaling.scale(normalized);
                let value = self.range.start()
                    + normalized_scaled * (self.range.end() - self.range.start());
                self.set_value(value);
            }
        }
    }
}
