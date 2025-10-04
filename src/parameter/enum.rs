use std::{fmt::Debug, str::FromStr};

use four_cc::FourCC;
use strum::IntoEnumIterator;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// An enum parameter descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumParameter {
    id: FourCC,
    name: &'static str,
    values: Vec<String>,
    default_index: usize,
}

impl EnumParameter {
    pub fn new<E: IntoEnumIterator + ToString + PartialEq>(
        id: FourCC,
        name: &'static str,
        default: E,
    ) -> Self {
        let values = E::iter().map(|v| v.to_string()).collect::<Vec<_>>();
        let default_index = E::iter().position(|r| r == default).unwrap_or(0);
        Self {
            id,
            name,
            values,
            default_index,
        }
    }

    pub fn default_value(&self) -> &String {
        &self.values[self.default_index]
    }

    pub fn clamp_value(&self, value: String) -> String {
        if self.values.contains(&value) {
            value
        } else {
            self.default_value().clone()
        }
    }

    pub fn normalize_value(&self, value: &String) -> f32 {
        if let Some(index) = self.values.iter().position(|v| v == value) {
            return index as f32 / (self.values.len() - 1) as f32;
        }
        0.0
    }

    pub fn denormalize_value(&self, normalized: f32) -> &String {
        assert!((0.0..=1.0).contains(&normalized));
        let index = (normalized * (self.values.len() - 1) as f32).round() as usize;
        &self.values[index]
    }
}

impl Parameter for EnumParameter {
    fn id(&self) -> FourCC {
        self.id
    }
    fn name(&self) -> &'static str {
        self.name
    }
    fn parameter_type(&self) -> ParameterType {
        ParameterType::Enum {
            values: self.values.clone(),
            default_index: self.default_index,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds an enum parameter value and its description.
#[derive(Debug, Clone, PartialEq)]
pub struct EnumParameterValue<T: Sized + Clone> {
    /// The current value of the parameter.
    value: T,
    /// The parameter's description and constraints.
    description: EnumParameter,
}

impl<T: Sized + FromStr + Clone + 'static> EnumParameterValue<T>
where
    <T as FromStr>::Err: Debug,
{
    pub fn from_description(description: EnumParameter) -> Self {
        let value = T::from_str(description.default_value()).unwrap();
        Self { value, description }
    }

    pub fn with_value(&self, value: T) -> Self {
        Self {
            value,
            description: self.description.clone(),
        }
    }

    #[inline(always)]
    pub fn value(&self) -> &T {
        &self.value
    }

    pub fn set_value(&mut self, value: T) {
        self.value = value;
    }

    pub fn description(&self) -> &EnumParameter {
        &self.description
    }

    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<T>() {
                    self.set_value(value.clone());
                } else if let Some(value_str) = raw.downcast_ref::<String>() {
                    if let Ok(value) = T::from_str(value_str) {
                        self.set_value(value);
                    } else {
                        log::warn!(
                            "Invalid string value for enum parameter '{}'",
                            self.description.id()
                        );
                    }
                } else {
                    log::warn!(
                        "Invalid value type for enum parameter '{}'",
                        self.description.id()
                    );
                }
            }
            ParameterValueUpdate::Normalized(normalized) => {
                let value_str = self
                    .description
                    .denormalize_value(normalized.clamp(0.0, 1.0));
                if let Ok(value) = T::from_str(value_str) {
                    self.set_value(value);
                }
            }
        }
    }
}

impl<T: Sized + FromStr + Clone + 'static> From<EnumParameter> for EnumParameterValue<T>
where
    <T as FromStr>::Err: Debug,
{
    fn from(description: EnumParameter) -> Self {
        Self::from_description(description)
    }
}
