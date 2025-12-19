use std::fmt::{Debug, Display};
use std::sync::Arc;

use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A boolean parameter descriptor.
#[derive(Clone)]
pub struct BooleanParameter {
    id: FourCC,
    name: &'static str,
    default: bool,
    #[allow(clippy::type_complexity)]
    value_to_string: Option<Arc<dyn Fn(bool) -> String + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<bool> + Send + Sync>>,
}

impl Debug for BooleanParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BooleanParameter")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("default", &self.default)
            .field("value_to_string", &self.value_to_string.is_some())
            .field("string_to_value", &self.string_to_value.is_some())
            .finish()
    }
}

impl BooleanParameter {
    /// Create a new boolean parameter descriptor.
    pub const fn new(id: FourCC, name: &'static str, default: bool) -> Self {
        Self {
            id,
            name,
            default,
            value_to_string: None,
            string_to_value: None,
        }
    }

    /// Optional custom conversion functions to convert the boolean value to a string
    /// and a string back to to a plain value.
    ///
    /// If strings cannot be parsed, the callback should return `None`.
    pub fn with_display<
        ValueToString: Fn(bool) -> String + Send + Sync + 'static,
        StringToValue: Fn(&str) -> Option<bool> + Send + Sync + 'static,
    >(
        mut self,
        value_to_string: ValueToString,
        string_to_value: StringToValue,
    ) -> Self {
        self.value_to_string = Some(Arc::new(value_to_string));
        self.string_to_value = Some(Arc::new(string_to_value));
        self
    }

    /// Create a raw, ParameterValueUpdate for this parameter.
    #[must_use]
    pub fn value_update(&self, value: bool) -> (FourCC, ParameterValueUpdate) {
        (self.id, ParameterValueUpdate::Raw(Arc::new(value)))
    }

    /// The parameter's identifier.
    pub const fn id(&self) -> FourCC {
        self.id
    }

    /// The parameter's default value.
    pub const fn default_value(&self) -> bool {
        self.default
    }

    /// Normalize the given plain value to a 0.0-1.0 range.
    pub const fn normalize_value(&self, value: bool) -> f32 {
        if value {
            1.0
        } else {
            0.0
        }
    }

    /// Denormalize a 0.0-1.0 ranged value to the corresponding plain value.
    pub fn denormalize_value(&self, normalized: f32) -> bool {
        assert!((0.0..=1.0).contains(&normalized));
        normalized >= 0.5
    }

    /// Convert the given plain value to a string, using a custom conversion function if provided.
    pub fn value_to_string(&self, value: bool) -> String {
        match &self.value_to_string {
            Some(f) => f(value),
            None => {
                if value {
                    "ON".to_string()
                } else {
                    "OFF".to_string()
                }
            }
        }
    }

    /// Convert the given string to a plain value, using a custom conversion function if provided.
    pub fn string_to_value(&self, string: &str) -> Option<bool> {
        let value = match &self.string_to_value {
            Some(f) => f(string.trim()),
            None => {
                let string = string.trim();
                if string.eq_ignore_ascii_case("ON") {
                    Some(true)
                } else if string.eq_ignore_ascii_case("OFF") {
                    Some(false)
                } else {
                    string.parse::<bool>().ok()
                }
            }
        }?;
        Some(value)
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
        ParameterType::Boolean
    }

    fn default_value(&self) -> f32 {
        self.normalize_value(self.default)
    }

    fn value_to_string(&self, normalized: f32, _include_unit: bool) -> String {
        let value = self.denormalize_value(normalized.clamp(0.0, 1.0));
        self.value_to_string(value)
    }

    fn string_to_value(&self, string: String) -> Option<f32> {
        let value = self.string_to_value(&string)?;
        Some(self.normalize_value(value))
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds a boolean parameter value and its description.
#[derive(Debug, Clone)]
pub struct BooleanParameterValue {
    /// The parameter's description and constraints.
    description: BooleanParameter,
    /// The current value of the parameter.
    value: bool,
}

impl BooleanParameterValue {
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: BooleanParameter) -> Self {
        let value = description.default_value();
        Self { value, description }
    }

    /// Create a new parameter value with the given value.
    pub fn with_value(mut self, value: bool) -> Self {
        self.value = value;
        self
    }

    /// Access the parameter value's description.
    pub fn description(&self) -> &BooleanParameter {
        &self.description
    }

    /// Access to the current value.
    #[inline(always)]
    pub fn value(&self) -> bool {
        self.value
    }

    /// Set a new value.
    pub fn set_value(&mut self, value: bool) {
        self.value = value;
    }

    /// Applies a parameter update.
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

impl Display for BooleanParameterValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description.value_to_string(self.value))
    }
}
