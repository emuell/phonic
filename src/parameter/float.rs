use std::ops::RangeInclusive;

use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A continuous (float) parameter descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatParameter {
    id: FourCC,
    name: &'static str,
    range: RangeInclusive<f32>,
    default: f32,
}

impl FloatParameter {
    pub fn new(id: FourCC, name: &'static str, range: RangeInclusive<f32>, default: f32) -> Self {
        assert!(range.contains(&default), "Invalid parameter default value");
        Self {
            id,
            name,
            range,
            default,
        }
    }

    pub fn range(&self) -> &RangeInclusive<f32> {
        &self.range
    }

    pub fn default_value(&self) -> f32 {
        self.default
    }

    pub fn clamp_value(&self, value: f32) -> f32 {
        value.clamp(*self.range.start(), *self.range.end())
    }

    pub fn normalize_value(&self, value: f32) -> f32 {
        (value - *self.range.start()) / (*self.range().end() - *self.range.start())
    }

    pub fn denormalize_value(&self, normalized: f32) -> f32 {
        assert!((0.0..=1.0).contains(&normalized));
        *self.range.start() + normalized * (*self.range().end() - *self.range.start())
    }
}

impl Parameter for FloatParameter {
    fn id(&self) -> FourCC {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn parameter_type(&self) -> ParameterType {
        ParameterType::Float {
            range: self.range.clone(),
            default: self.default,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds a float parameter value and its description.
#[derive(Debug, Clone, PartialEq)]
pub struct FloatParameterValue {
    /// The current value of the parameter.
    value: f32,
    /// The parameter's description and constraints.
    description: FloatParameter,
}

impl FloatParameterValue {
    pub fn from_description(description: FloatParameter) -> Self {
        let value = description.default_value();
        Self { value, description }
    }

    pub fn with_value(&self, value: f32) -> Self {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        Self {
            value,
            description: self.description.clone(),
        }
    }

    #[inline(always)]
    pub fn value(&self) -> &f32 {
        &self.value
    }

    pub fn set_value(&mut self, value: f32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value = value;
    }

    pub fn set_value_clamped(&mut self, value: f32) {
        self.value = self.description.clamp_value(value);
    }

    pub fn description(&self) -> &FloatParameter {
        &self.description
    }

    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<f32>() {
                    self.set_value_clamped(*value);
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

impl From<FloatParameter> for FloatParameterValue {
    fn from(description: FloatParameter) -> Self {
        Self::from_description(description)
    }
}
