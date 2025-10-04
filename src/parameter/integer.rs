use std::ops::RangeInclusive;

use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A discrete (integer) parameter descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegerParameter {
    id: FourCC,
    name: &'static str,
    range: RangeInclusive<i32>,
    default: i32,
}

impl IntegerParameter {
    pub fn new(id: FourCC, name: &'static str, range: RangeInclusive<i32>, default: i32) -> Self {
        assert!(range.contains(&default), "Invalid parameter default value");
        Self {
            id,
            name,
            range,
            default,
        }
    }

    pub fn range(&self) -> &RangeInclusive<i32> {
        &self.range
    }

    pub fn default_value(&self) -> i32 {
        self.default
    }

    pub fn clamp_value(&self, value: i32) -> i32 {
        value.clamp(*self.range.start(), *self.range.end())
    }

    pub fn normalize_value(&self, value: i32) -> f32 {
        (value as f32 - *self.range.start() as f32)
            / (*self.range.end() as f32 - *self.range.start() as f32)
    }

    pub fn denormalize_value(&self, normalized: f32) -> i32 {
        assert!((0.0..=1.0).contains(&normalized));
        let value = *self.range.start() as f32
            + normalized * (*self.range.end() as f32 - *self.range.start() as f32);
        value.round() as i32
    }
}

impl Parameter for IntegerParameter {
    fn id(&self) -> FourCC {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn parameter_type(&self) -> ParameterType {
        ParameterType::Integer {
            range: self.range.clone(),
            default: self.default,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds an integer parameter value and its description.
#[derive(Debug, Clone, PartialEq)]
pub struct IntegerParameterValue {
    /// The current value of the parameter.
    value: i32,
    /// The parameter's description and constraints.
    description: IntegerParameter,
}

impl IntegerParameterValue {
    pub fn from_description(description: IntegerParameter) -> Self {
        let value = description.default_value();
        Self { value, description }
    }

    pub fn with_value(&self, value: i32) -> Self {
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
    pub fn value(&self) -> &i32 {
        &self.value
    }

    pub fn set_value(&mut self, value: i32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value = value;
    }

    pub fn set_value_clamped(&mut self, value: i32) {
        self.value = self.description.clamp_value(value);
    }

    pub fn description(&self) -> &IntegerParameter {
        &self.description
    }

    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<i32>() {
                    self.set_value_clamped(*value);
                } else {
                    log::warn!(
                        "Invalid value type for integer parameter '{}'",
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

impl From<IntegerParameter> for IntegerParameterValue {
    fn from(description: IntegerParameter) -> Self {
        Self::from_description(description)
    }
}
