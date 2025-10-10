use std::fmt::Debug;

use crate::utils::{db_to_linear, linear_to_db};

// -------------------------------------------------------------------------------------------------

/// Effect parameter scaling for float parameters, applied to convert normalized UI or automation
/// values to the internal values.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum ParameterScaling {
    #[default]
    /// Linear scaling: `y = x` (no transformation applied)
    Linear,

    /// Exponential scaling: `y = x^factor`
    /// Factor must be > 0.0.
    ///
    /// Values < 1.0 create a curve that rises slowly at first then quickly.
    /// Values > 1.0 create a curve that rises quickly at first then slowly.
    ///
    /// Factors between 2.0 - 3.0 are typically used for Hz (e.g. filter cutoffs).
    ///
    Exponential(f32),

    /// Decibel scaling: maps normalized value to dB range, then converts to linear gain
    /// Parameters are (min_db, max_db). max_db must be > min_db.
    ///
    /// The normalized value \[0, 1\] is mapped to the dB range \[min_db, max_db\], then converted
    /// to linear gain. The scaling returns a 0-1 position within the linear gain range.
    ///
    /// Allows storing/applying a linear gain value while displaying a dB value with proper scaling.
    /// The internal value range should use `db_to_lin(minDb)..=db_to_lin(maxDb)`.
    Decibel(f32, f32),

    /// Sigmoid (S-curve) scaling: `y = 1 / (1 + e^(-steepness * (x - 0.5)))`
    /// Steepness must be > 0.0.
    ///
    /// Creates an S-shaped curve, normalized to map \[0,1\] -> \[0,1\]. Higher steepness values create
    /// a sharper transition around the midpoint.
    Sigmoid(f32),
}

impl ParameterScaling {
    /// Apply scaling to a normalized f32 value.
    pub fn scale(&self, value: f32) -> f32 {
        assert!(
            (0.0..=1.0).contains(&value),
            "Expecting a normalized value here"
        );
        match self {
            ParameterScaling::Linear => value,
            ParameterScaling::Exponential(factor) => {
                // Exponential curve: y = x^factor
                value.powf(*factor)
            }
            ParameterScaling::Sigmoid(steepness) => {
                // Sigmoid curve: y = 1 / (1 + e^(-steepness * (x - 0.5)))
                let sigmoid = |x: f32| 1.0 / (1.0 + (-steepness * (x - 0.5)).exp());
                let y = sigmoid(value);
                let y_min = sigmoid(0.0);
                let y_max = sigmoid(1.0);
                (y - y_min) / (y_max - y_min)
            }
            ParameterScaling::Decibel(min_db, max_db) => {
                // Map normalized value to dB range
                let db_value = min_db + value * (max_db - min_db);
                // Convert dB to linear gain
                let linear_gain = db_to_linear(db_value);
                // Map to [0,1] range relative to [db_to_lin(min_db), db_to_lin(max_db)]
                let (min_linear, max_linear) = (db_to_linear(*min_db), db_to_linear(*max_db));
                (linear_gain - min_linear) / (max_linear - min_linear)
            }
        }
    }

    /// Apply inverse scaling to a normalized f32 value.
    pub fn unscale(&self, value: f32) -> f32 {
        assert!(
            (0.0..=1.0).contains(&value),
            "Expecting a normalized value here"
        );
        match self {
            ParameterScaling::Linear => value,
            ParameterScaling::Exponential(factor) => {
                // Inverse of exponential: x = y^(1/factor)
                let factor = factor.abs().max(0.001);
                value.powf(1.0 / factor)
            }
            ParameterScaling::Sigmoid(steepness) => {
                // Inverse of sigmoid: x = 0.5 - ln((1/y) - 1) / steepness
                let sigmoid = |x: f32| 1.0 / (1.0 + (-steepness * (x - 0.5)).exp());
                let y_min = sigmoid(0.0);
                let y_max = sigmoid(1.0);
                let y = value * (y_max - y_min) + y_min;
                let y_clamped = y.clamp(0.0001, 0.9999); // Avoid log(0)
                0.5 - ((1.0 / y_clamped) - 1.0).ln() / steepness
            }
            ParameterScaling::Decibel(min_db, max_db) => {
                // value is a 0-1 position in the [db_to_lin(min_db), db_to_lin(max_db)] range
                let (min_linear, max_linear) = (db_to_linear(*min_db), db_to_linear(*max_db));
                let linear_gain = min_linear + value * (max_linear - min_linear);
                // Convert linear gain to dB
                let db_value = linear_to_db(linear_gain);
                // Map dB back to normalized range
                (db_value - min_db) / (max_db - min_db)
            }
        }
    }

    pub(crate) const fn validate(&self) {
        match self {
            ParameterScaling::Linear => {}
            ParameterScaling::Exponential(factor) => {
                assert!(
                    *factor > 0.0,
                    "Invalid exponential parameter scaling factor (must be > 0)"
                );
            }
            ParameterScaling::Sigmoid(steepness) => {
                assert!(
                    *steepness > 0.0,
                    "Invalid sigmoid parameter scaling steepness (must be > 0)",
                );
            }
            ParameterScaling::Decibel(min_db, max_db) => {
                assert!(
                    *min_db < *max_db,
                    "Invalid decibel parameter scaling range (min_db must be < max_db)"
                );
            }
        }
    }
}
