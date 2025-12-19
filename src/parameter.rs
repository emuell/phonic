//! Effect parameter descriptors and value wrappers.

use std::{any::Any, fmt::Debug, sync::Arc};

use four_cc::FourCC;

// -------------------------------------------------------------------------------------------------

/// Describes polarity of a [`Parameter`] for visual representations in a UI.
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterPolarity {
    /// A continous, ranged value.
    Unipolar,
    /// A continous, symetrically ranged value, centered around 0.
    Bipolar,
}

// -------------------------------------------------------------------------------------------------

/// Describes the type of a [`Parameter`] to e.g. select a proper visual representation in a UI.
///
/// Parameter UIs and/or automation, access parameter values as *normalized* float values only:
/// - To show values as human readable strings, use the [`value_to_string`](Parameter::value_to_string)
/// and [`string_to_value`](Parameter::string_to_value) functions.
/// - Use the `values` property for enum parameters to visualize selected and available choices.
/// Enum values have a `step` of `1.0 / values.len()`.
/// - The `step` property of float and integer parameters can be used in UIs to quantize normalized
/// value changes in sliders.
/// - The `polarity` property may be used to change visual appearence of sliders.
///
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterType {
    /// A continuous floating-point value.
    Float {
        /// Quantize step. Usually 0.0 for floats, which means there's no step.
        step: f32,
        /// Display polarity.
        polarity: ParameterPolarity,
    },
    /// A discrete integer value.
    Integer {
        /// Quantize step to match target integer values: `1.0 / (range.end - range.start)`
        /// in normalized values.
        step: f32,
        /// Display polarity.
        polarity: ParameterPolarity,
    },
    /// A choice from a list of strings (an enum).
    Enum { values: Vec<String> },
    /// A boolean toggle.
    Boolean,
}

// -------------------------------------------------------------------------------------------------

/// Describes a single parameter in a [`Effect`](super::Effect) or [`Generator`](super::Generator)
/// for use in UIs automation, and can be `Send` and `Sync`ed across threads.
///
/// Note that parameter descriptions don't hold the actual parameter values, just the default values.
/// The effect or generator processor owns the actual value.
pub trait Parameter: Debug + Send + Sync {
    /// The unique id of the parameter.
    fn id(&self) -> FourCC;

    /// The name of the parameter.
    fn name(&self) -> &'static str;

    /// The parameter type.
    fn parameter_type(&self) -> ParameterType;

    /// Default value of parameter, expressed as **normalized** floating point value.
    fn default_value(&self) -> f32;

    /// Convert the given **normalized** floating point value to a string value.
    fn value_to_string(&self, value: f32, include_unit: bool) -> String;
    /// Convert the given string value to a **normalized** floating point value.
    /// Returns `None` when conversion failed, else a valid normalized value.
    fn string_to_value(&self, string: String) -> Option<f32>;
}

/// Allows creating `dyn `[`Parameter`] clones.
pub trait ClonableParameter: Parameter {
    /// Create a dyn Parameter clone, wrapped into a box.
    fn dyn_clone(&self) -> Box<dyn ClonableParameter>;
    /// Cast parameter to any.
    fn as_any(&self) -> &dyn Any;
}

impl<P: Parameter> ClonableParameter for P
where
    P: Clone + 'static,
{
    fn dyn_clone(&self) -> Box<dyn ClonableParameter> {
        Box::new(Self::clone(self))
    }

    fn as_any(&self) -> &dyn Any {
        self as &dyn Any
    }
}

// -------------------------------------------------------------------------------------------------

/// An update for a [`Parameter`]'s value, consumed by [`Effect`](super::Effect)s or
/// [`Generator`](crate::Generator) in audio time.
#[derive(Debug, Clone)]
pub enum ParameterValueUpdate {
    /// Raw, type-erased internal value (f32, i32, some Enum or boolean).
    Raw(Arc<dyn Any + Send + Sync>),
    /// A float value in range `0.0..=1.0`.
    Normalized(f32),
}

// -------------------------------------------------------------------------------------------------

mod float;
pub use float::{FloatParameter, FloatParameterValue};

mod smoothed;
pub use smoothed::SmoothedParameterValue;

mod integer;
pub use integer::{IntegerParameter, IntegerParameterValue};

mod r#enum;
pub use r#enum::{EnumParameter, EnumParameterValue};

mod boolean;
pub use boolean::{BooleanParameter, BooleanParameterValue};

mod scaling;
pub use scaling::ParameterScaling;
