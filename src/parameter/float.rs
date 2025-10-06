use std::{
    fmt::{Debug, Display},
    ops::RangeInclusive,
    sync::Arc,
};

use four_cc::FourCC;

use super::{Parameter, ParameterType, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A continuous (float) parameter descriptor.
#[derive(Clone)]
pub struct FloatParameter {
    id: FourCC,
    name: &'static str,
    range: RangeInclusive<f32>,
    default: f32,
    unit: &'static str,
    #[allow(clippy::type_complexity)]
    value_to_string: Option<Arc<dyn Fn(f32) -> String + Send + Sync>>,
    #[allow(clippy::type_complexity)]
    string_to_value: Option<Arc<dyn Fn(&str) -> Option<f32> + Send + Sync>>,
}

impl Debug for FloatParameter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FloatParameter")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("range", &self.range)
            .field("default", &self.default)
            .field("value_to_string", &self.value_to_string.is_some())
            .field("string_to_value", &self.string_to_value.is_some())
            .finish()
    }
}

impl FloatParameter {
    /// Create a new float parameter descriptor.
    pub const fn new(
        id: FourCC,
        name: &'static str,
        range: RangeInclusive<f32>,
        default: f32,
    ) -> Self {
        assert!(
            default >= *range.start() && default <= *range.end(),
            "Invalid parameter default value"
        );
        Self {
            id,
            name,
            range,
            default,
            unit: "",
            value_to_string: None,
            string_to_value: None,
        }
    }

    /// Optional unit for string displays.
    pub const fn with_unit(mut self, unit: &'static str) -> Self {
        self.unit = unit;
        self
    }

    /// Optional custom conversion functions to convert a plain value to a string and string
    /// to a plain value.
    ///
    /// Returned strings should not contain a unit, if a unit already was set for this parameter.
    ///
    /// If strings cannot be parsed, the callback should return `None`. returned values will be
    /// clamped automatically, so the converted does not need to clamp them.
    pub fn with_display<
        ValueToString: Fn(f32) -> String + Send + Sync + 'static,
        StringToValue: Fn(&str) -> Option<f32> + Send + Sync + 'static,
    >(
        mut self,
        value_to_string: ValueToString,
        string_to_value: StringToValue,
    ) -> Self {
        self.value_to_string = Some(Arc::new(value_to_string));
        self.string_to_value = Some(Arc::new(string_to_value));
        self
    }

    /// The parameter's value range.
    pub fn range(&self) -> &RangeInclusive<f32> {
        &self.range
    }

    /// The parameter's default value.
    pub fn default_value(&self) -> f32 {
        self.default
    }

    /// Clamp the given plain value to the parameter's range.
    pub fn clamp_value(&self, value: f32) -> f32 {
        value.clamp(*self.range.start(), *self.range.end())
    }

    /// Normalize the given plain value to a 0.0-1.0 range.
    pub fn normalize_value(&self, value: f32) -> f32 {
        (value - *self.range.start()) / (*self.range().end() - *self.range.start())
    }

    /// Denormalize a 0.0-1.0 ranged value to the corresponding plain value.
    pub fn denormalize_value(&self, normalized: f32) -> f32 {
        assert!((0.0..=1.0).contains(&normalized));
        *self.range.start() + normalized * (*self.range().end() - *self.range.start())
    }

    /// Convert the given plain value to a string, using a custom conversion function if provided.
    pub fn value_to_string(&self, value: f32, include_unit: bool) -> String {
        match (&self.value_to_string, include_unit && !self.unit.is_empty()) {
            (Some(f), true) => format!("{} {}", f(value), self.unit),
            (Some(f), false) => f(value),
            (None, true) => format!("{:.2} {}", value, self.unit),
            (None, false) => format!("{:.2}", value),
        }
    }

    /// Convert the given string to a plain value, using a custom conversion function if provided.
    pub fn string_to_value(&self, string: &str) -> Option<f32> {
        let value = match &self.string_to_value {
            Some(f) => f(string.trim()),
            None => string.trim().trim_end_matches(self.unit).parse().ok(),
        }?;
        Some(self.clamp_value(value))
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

    fn normalized_value_to_string(&self, normalized: f32, include_unit: bool) -> String {
        let value = self.denormalize_value(normalized.clamp(0.0, 1.0));
        self.value_to_string(value, include_unit)
    }
    
    fn string_to_normalized_value(&self, string: String) -> Option<f32> {
        let value = self.string_to_value(&string)?;
        Some(self.normalize_value(value))
    }
}

// -------------------------------------------------------------------------------------------------

/// Holds a float parameter value and its description.
#[derive(Debug, Clone)]
pub struct FloatParameterValue {
    /// The parameter's description and constraints.
    description: FloatParameter,
    /// The current value of the parameter.
    value: f32,
}

impl FloatParameterValue {
    /// Create a new parameter value with the given parameter description, initialized to the
    /// parameter's default value.
    pub fn from_description(description: FloatParameter) -> Self {
        let value = description.default_value();
        Self { value, description }
    }

    /// Create a new parameter value with the given value.
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

    /// Access the parameter value's description.
    pub fn description(&self) -> &FloatParameter {
        &self.description
    }

    /// Access to the current value.
    #[inline(always)]
    pub fn value(&self) -> f32 {
        self.value
    }

    /// Set a new value.
    pub fn set_value(&mut self, value: f32) {
        assert!(
            self.description.range().contains(&value),
            "Value out of bounds"
        );
        self.value = value;
    }

    /// Set a new value, clamping the given value into the parameter's value bounds if necessary.
    pub fn set_value_clamped(&mut self, value: f32) {
        self.value = self.description.clamp_value(value);
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

impl Display for FloatParameterValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let include_unit = true;
        f.write_str(&self.description.value_to_string(self.value, include_unit))
    }
}
