//! Effect parameter descriptors and value wrappers.

use std::{any::Any, fmt::Debug, ops::RangeInclusive};

use four_cc::FourCC;

// -------------------------------------------------------------------------------------------------

/// Describes the type, default value and range of a [`Parameter`].
#[derive(Debug, Clone, PartialEq)]
pub enum ParameterType {
    /// A continuous floating-point value within a given range.
    Float {
        range: RangeInclusive<f32>,
        default: f32,
    },
    /// A discrete integer value within a given range.
    Integer {
        range: RangeInclusive<i32>,
        default: i32,
    },
    /// A choice from a list of strings (for enums).
    Enum {
        values: Vec<String>,
        default_index: usize,
    },
    /// A boolean toggle.
    Boolean { default: bool },
}

// -------------------------------------------------------------------------------------------------

/// Describes a single parameter of an [`Effect`](super::Effect) for use in UIs or for automation.
pub trait Parameter: Debug {
    /// The unique id of the parameter.
    fn id(&self) -> FourCC;
    /// The name of the parameter.
    fn name(&self) -> &'static str;
    /// The type and range of the parameter.
    fn parameter_type(&self) -> ParameterType;

    /// Convert the given normalized parameter value to a string value.
    fn normalized_value_to_string(&self, value: f32, include_unit: bool) -> String;
    /// Convert the given string value to a normalized parameter value. Returns `None`
    /// when conversion failed, else a valid normalized value.
    fn string_to_normalized_value(&self, string: String) -> Option<f32>;
}

/// Allows creating `dyn Parameter` clones.
pub trait ClonableParameter: Parameter {
    /// Create a dyn Parameter clone, wrapped into a box.  
    fn dyn_clone(&self) -> Box<dyn Parameter>;
}

impl<P> ClonableParameter for P
where
    P: Parameter + Clone + 'static,
{
    fn dyn_clone(&self) -> Box<dyn Parameter> {
        Box::new(Self::clone(self))
    }
}

// -------------------------------------------------------------------------------------------------

/// An update for a [`Parameter`]'s value, consumed by [`Effect`](super::Effect)s in audio time.
#[derive(Debug)]
pub enum ParameterValueUpdate {
    /// Raw, type-erased internal value (f32, i32, some Enum or boolean).
    Raw(Box<dyn Any + Send + Sync>),
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
