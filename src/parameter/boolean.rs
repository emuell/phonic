use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A boolean parameter descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct BooleanParameter {
    id: FourCC,
    name: &'static str,
    default: bool,
}

impl BooleanParameter {
    pub fn new(id: FourCC, name: &'static str, default: bool) -> Self {
        Self { id, name, default }
    }

    pub fn default_value(&self) -> bool {
        self.default
    }

    pub fn clamp_value(&self, value: bool) -> bool {
        value
    }

    pub fn normalize_value(&self, value: bool) -> f32 {
        if value {
            1.0
        } else {
            0.0
        }
    }

    pub fn denormalize_value(&self, normalized: f32) -> bool {
        assert!((0.0..=1.0).contains(&normalized));
        normalized >= 0.5
    }
}

impl Parameter for BooleanParameter {
    fn id(&self) -> FourCC {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn parameter_type(&self) -> ParameterType {
        ParameterType::Boolean {
            default: self.default,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds a boolean parameter value and its description.
#[derive(Debug, Clone, PartialEq)]
pub struct BooleanParameterValue {
    /// The current value of the parameter.
    value: bool,
    /// The parameter's description and constraints.
    description: BooleanParameter,
}

impl BooleanParameterValue {
    pub fn from_description(description: BooleanParameter) -> Self {
        let value = description.default_value();
        Self { value, description }
    }

    pub fn with_value(&self, value: bool) -> Self {
        Self {
            value,
            description: self.description.clone(),
        }
    }

    #[inline(always)]
    pub fn value(&self) -> &bool {
        &self.value
    }

    pub fn set_value(&mut self, value: bool) {
        self.value = value;
    }

    pub fn description(&self) -> &BooleanParameter {
        &self.description
    }

    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<bool>() {
                    self.set_value(*value);
                } else {
                    log::warn!(
                        "Invalid value type for boolean parameter '{}'",
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

impl From<BooleanParameter> for BooleanParameterValue {
    fn from(description: BooleanParameter) -> Self {
        Self::from_description(description)
    }
}
