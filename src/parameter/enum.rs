use std::{fmt::Debug, fmt::Display, str::FromStr, sync::Arc};

use four_cc::FourCC;
use strum::IntoEnumIterator;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// An enum parameter descriptor.
#[derive(Clone)]
pub struct EnumParameter {
    id: FourCC,
    name: &'static str,
    values: Vec<String>,
    default_index: usize,
    #[allow(clippy::type_complexity)]
    value_to_string: Option<Arc<dyn Fn(&String) -> String + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<String> + Send + Sync>>,
}

impl Debug for EnumParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnumParameter")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("values", &self.values)
            .field("default_index", &self.default_index)
            .field("value_to_string", &self.value_to_string.is_some())
            .field("string_to_value", &self.string_to_value.is_some())
            .finish()
    }
}

impl EnumParameter {
    /// Create a new enum parameter descriptor.
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
            value_to_string: None,
            string_to_value: None,
        }
    }

    /// The parameter's default value.
    pub fn default_value(&self) -> &String {
        &self.values[self.default_index]
    }

    /// Clamp the given plain value to the parameter's range of valid enum values.
    pub fn clamp_value(&self, value: String) -> String {
        if self.values.contains(&value) {
            value
        } else {
            self.default_value().clone()
        }
    }

    /// Normalize the given plain value to a 0.0-1.0 range.
    pub fn normalize_value(&self, value: &String) -> f32 {
        if let Some(index) = self.values.iter().position(|v| v == value) {
            return index as f32 / (self.values.len() - 1) as f32;
        }
        0.0
    }

    /// Denormalize a 0.0-1.0 ranged value to the corresponding plain value.
    pub fn denormalize_value(&self, normalized: f32) -> &String {
        assert!((0.0..=1.0).contains(&normalized));
        let index = (normalized * (self.values.len() - 1) as f32).round() as usize;
        &self.values[index]
    }

    /// Optional custom conversion functions to convert a plain value to a string and string
    /// to a plain value.
    ///
    /// If strings cannot be parsed, the callback should return `None`. returned values will be
    /// clamped automatically, so the converted does not need to clamp them.
    pub fn with_display<
        ValueToString: Fn(&String) -> String + Send + Sync + 'static,
        StringToValue: Fn(&str) -> Option<String> + Send + Sync + 'static,
    >(
        mut self,
        value_to_string: ValueToString,
        string_to_value: StringToValue,
    ) -> Self {
        self.value_to_string = Some(Arc::new(value_to_string));
        self.string_to_value = Some(Arc::new(string_to_value));
        self
    }

    /// Convert the given plain value to a string, using a custom conversion function if provided.
    pub fn value_to_string(&self, value: &String) -> String {
        match &self.value_to_string {
            Some(f) => f(value),
            None => value.clone(),
        }
    }

    /// Convert the given string to a plain value, using a custom conversion function if provided.
    pub fn string_to_value(&self, string: &str) -> Option<String> {
        let value = match &self.string_to_value {
            Some(f) => f(string.trim()),
            None => Some(string.trim().to_string()),
        }?;
        Some(self.clamp_value(value))
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
        }
    }

    fn default_value(&self) -> f32 {
        self.default_index as f32 / (self.values.len() - 1) as f32
    }

    fn value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        let value = self.denormalize_value(normalized.clamp(0.0, 1.0));
        self.value_to_string(value)
    }

    fn string_to_value(&self, string: String) -> Option<f32> {
        let value = self.string_to_value(&string)?;
        Some(self.normalize_value(&value))
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds an enum parameter value and its description.
#[derive(Debug, Clone)]
pub struct EnumParameterValue<T: Clone> {
    /// The parameter's description and constraints.
    description: EnumParameter,
    /// The current value of the parameter.
    value: T,
}

impl<T: Copy + Clone + FromStr + ToString + 'static> EnumParameterValue<T>
where
    <T as FromStr>::Err: Debug,
{
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: EnumParameter) -> Self {
        let value = T::from_str(description.default_value()).unwrap();
        Self { value, description }
    }

    /// Create a new parameter value with the given value.
    pub fn with_value(mut self, value: T) -> Self {
        self.value = value;
        self
    }

    /// Access the parameter value's description.
    pub fn description(&self) -> &EnumParameter {
        &self.description
    }

    /// Access to the current value.
    #[inline(always)]
    pub fn value(&self) -> T {
        self.value
    }

    /// Set a new value.
    pub fn set_value(&mut self, value: T) {
        self.value = value;
    }

    /// Applies a parameter update.
    pub fn apply_update(&mut self, update: &ParameterValueUpdate) {
        match update {
            ParameterValueUpdate::Raw(raw) => {
                if let Some(value) = raw.downcast_ref::<T>() {
                    self.set_value(*value);
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

impl<T: Clone + FromStr + ToString> Display for EnumParameterValue<T>
where
    <T as FromStr>::Err: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value_str = self.value.to_string();
        f.write_str(&self.description.value_to_string(&value_str))
    }
}
