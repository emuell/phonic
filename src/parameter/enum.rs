use std::{fmt::Debug, fmt::Display, str::FromStr, sync::Arc};

use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// An enum parameter descriptor.
#[derive(Clone)]
pub struct EnumParameter {
    id: FourCC,
    name: &'static str,
    values: &'static [&'static str],
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
    /// Create a new enum parameter descriptor from a static array of string slices.
    ///
    /// This constructor is `const` compatible, allowing enum parameters to be defined as constants.
    ///
    /// See also `Self::from_enum` for an alternative non const constructor which uses existing rust
    /// enum definiitions via the strum crate.
    pub const fn new(
        id: FourCC,
        name: &'static str,
        values: &'static [&'static str],
        default_index: usize,
    ) -> Self {
        assert!(!values.is_empty(), "EnumParameter values cannot be empty");
        assert!(
            default_index < values.len(),
            "Default index out of bounds for EnumParameter"
        );
        Self {
            id,
            name,
            values,
            default_index,
            value_to_string: None,
            string_to_value: None,
        }
    }

    /// Create a new enum parameter descriptor from a Rust enum type that implements
    /// `strum::VariantNames`, `PartialEq` and `ToString`.
    pub fn from_enum<E: strum::VariantNames + PartialEq + ToString>(
        id: FourCC,
        name: &'static str,
        default_value: E,
    ) -> Self {
        let values = E::VARIANTS;
        let default_string = default_value.to_string();
        let default_index = values
            .iter()
            .position(|v| v == &default_string)
            .expect("Failed to resolve enum default value");
        Self::new(id, name, values, default_index)
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

    /// Create a raw, debug validated ParameterValueUpdate from an enum value for this parameter.
    #[must_use]
    pub fn value_update<E: PartialEq + ToString + Send + Sync + 'static>(
        &self,
        value: E,
    ) -> (FourCC, ParameterValueUpdate) {
        debug_assert!(
            self.values.iter().any(|v| v == &value.to_string()),
            "Enum value for parameter '{}' is not one of '{:?}', but is {}",
            self.id,
            self.values,
            value.to_string()
        );
        (self.id, ParameterValueUpdate::Raw(Arc::new(value)))
    }

    /// Create a raw, debug validated ParameterValueUpdate from an index for this parameter.
    #[must_use]
    pub fn value_update_index(&self, index: usize) -> (FourCC, ParameterValueUpdate) {
        debug_assert!(
            index < self.values.len(),
            "Enum value for parameter '{}' must be < {}, but is {}",
            self.id,
            self.values.len(),
            index
        );
        (self.id, ParameterValueUpdate::Raw(Arc::new(index)))
    }

    /// The parameter's identifier.
    pub const fn id(&self) -> FourCC {
        self.id
    }

    /// The parameter's choices.
    pub const fn values(&self) -> &'static [&'static str] {
        self.values
    }

    /// The parameter's default value.
    pub const fn default_value(&self) -> &str {
        self.values[self.default_index]
    }

    /// Clamp the given plain value to the parameter's set of valid enum values.
    pub fn clamp_value<'a>(&self, value: &'a str) -> &'a str {
        if self.values.contains(&value) {
            value
        } else {
            self.values[self.default_index]
        }
    }

    /// Normalize the given plain value to a 0.0-1.0 range.
    pub fn normalize_value(&self, value: &str) -> f32 {
        if let Some(index) = self.values.iter().position(|v| v == &value) {
            return index as f32 / (self.values.len() - 1) as f32;
        }
        0.0
    }

    /// Denormalize a 0.0-1.0 ranged value to the corresponding plain value.
    pub fn denormalize_value(&self, normalized: f32) -> &str {
        assert!((0.0..=1.0).contains(&normalized));
        let index = (normalized * (self.values.len() - 1) as f32).round() as usize;
        self.values[index]
    }

    /// Convert the given plain value to a string, using a custom conversion function if provided.
    pub fn value_to_string(&self, value: &str) -> String {
        match &self.value_to_string {
            Some(f) => f(&value.to_owned()),
            None => value.to_owned(),
        }
    }

    /// Convert the given string to a plain value, using a custom conversion function if provided.
    pub fn string_to_value(&self, string: &str) -> Option<String> {
        match &self.string_to_value {
            Some(f) => f(string.trim()).map(|v| self.clamp_value(&v).to_owned()),
            None => Some(self.clamp_value(string.trim()).to_owned()),
        }
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
            values: self.values.iter().map(|v| v.to_string()).collect(),
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

    fn dyn_clone(&self) -> Box<dyn Parameter> {
        Box::new(self.clone())
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

impl<T: Clone> EnumParameterValue<T>
where
    T: FromStr + 'static,
{
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: EnumParameter) -> Self
    where
        <T as FromStr>::Err: Debug,
    {
        let value = T::from_str(description.default_value())
            .expect("Failed to convert default enum string value to the actual enum type");
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
        self.value.clone()
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
                    self.value = value.clone();
                } else if let Some(value_str) = raw.downcast_ref::<String>() {
                    if let Ok(value) = T::from_str(value_str) {
                        self.value = value;
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
                } else {
                    log::warn!(
                        "Invalid value string for enum parameter '{}'",
                        self.description.id()
                    );
                }
            }
        }
    }
}

impl<T: Clone> Display for EnumParameterValue<T>
where
    T: ToString,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value_str = self.value.to_string();
        f.write_str(&self.description.value_to_string(&value_str))
    }
}
