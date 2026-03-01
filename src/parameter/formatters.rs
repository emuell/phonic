//! Pre-built display formatter pairs for common parameter types.

use crate::utils::{db_to_linear, linear_to_db};

// -------------------------------------------------------------------------------------------------

/// A `(value_to_string, string_to_value)` function pair for
/// [`FloatParameter`](super::FloatParameter) display.
pub type FloatFormatter = (fn(f32) -> String, fn(&str) -> Option<f32>);

/// A `(value_to_string, string_to_value)` function pair for
/// [`IntegerParameter`](super::IntegerParameter) display.
pub type IntegerFormatter = (fn(i32) -> String, fn(&str) -> Option<i32>);

/// A `(value_to_string, string_to_value)` function pair for
/// [`EnumParameter`](super::EnumParameter) display.
pub type EnumFormatter = (fn(&str) -> String, fn(&str) -> Option<String>);

/// A `(value_to_string, string_to_value)` function pair for
/// [`BooleanParameter`](super::BooleanParameter) display.
pub type BooleanFormatter = (fn(bool) -> String, fn(&str) -> Option<bool>);

// -------------------------------------------------------------------------------------------------

fn percent_to_string(v: f32) -> String {
    format!("{:.2} %", v * 100.0)
}

fn percent_from_string(s: &str) -> Option<f32> {
    let s = s
        .trim()
        .trim_end_matches(|c: char| c == '%' || c.is_whitespace());
    s.parse::<f32>().ok().map(|v| v / 100.0)
}

/// Formats a normalized float value as a percentage.
pub const PERCENT: FloatFormatter = (percent_to_string, percent_from_string);

// -------------------------------------------------------------------------------------------------

fn gain_to_string(v: f32) -> String {
    let db = linear_to_db(v);
    if db <= -60.0 {
        "-INF dB".to_string()
    } else {
        format!("{:.2} dB", db)
    }
}

fn gain_from_string(s: &str) -> Option<f32> {
    if s.trim().eq_ignore_ascii_case("-inf") || s.trim().eq_ignore_ascii_case("inf") {
        Some(db_to_linear(-60.0))
    } else {
        let s = s.trim_start().trim_end_matches(|c: char| {
            c.eq_ignore_ascii_case(&'d') || c.eq_ignore_ascii_case(&'b') || c.is_whitespace()
        });
        s.parse::<f32>().ok().map(db_to_linear)
    }
}

/// Formats a linear gain value as dB (e.g. `"-INF dB"`, `"6.00 dB"`).
pub const GAIN: FloatFormatter = (gain_to_string, gain_from_string);

// -------------------------------------------------------------------------------------------------

fn decibels_to_string(v: f32) -> String {
    if v <= -60.0 {
        "-INF dB".to_string()
    } else {
        format!("{:.2} dB", v)
    }
}

fn decibels_from_string(s: &str) -> Option<f32> {
    if s.trim().eq_ignore_ascii_case("-inf") || s.trim().eq_ignore_ascii_case("inf") {
        Some(-60.0)
    } else {
        let s = s.trim_start().trim_end_matches(|c: char| {
            c.eq_ignore_ascii_case(&'d') || c.eq_ignore_ascii_case(&'b') || c.is_whitespace()
        });
        s.parse::<f32>().ok()
    }
}

/// Formats a dB value directly (e.g. `"-INF dB"` at -60 dB or below, `"-12.00 dB"`).
///
/// Unlike [`GAIN`], this formatter expects values already in dB (not linear).
pub const DECIBELS: FloatFormatter = (decibels_to_string, decibels_from_string);

// -------------------------------------------------------------------------------------------------

fn pan_to_string(v: f32) -> String {
    let v = v * 50.0;
    if v.abs() < 0.1 {
        "C".to_string()
    } else if v < 0.0 {
        format!("{:.0}L", v.abs())
    } else {
        format!("{:.0}R", v)
    }
}

pub fn pan_from_string(s: &str) -> Option<f32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("c") {
        Some(0.0)
    } else {
        let last_char = s.trim_end().chars().last().unwrap_or(' ');
        if last_char.eq_ignore_ascii_case(&'l') {
            let s = s
                .trim_start()
                .trim_end_matches(|c: char| c.eq_ignore_ascii_case(&'l') || c.is_whitespace());
            s.parse::<f32>().ok().map(|v| -v / 50.0)
        } else if last_char.eq_ignore_ascii_case(&'r') {
            let s = s
                .trim_start()
                .trim_end_matches(|c: char| c.eq_ignore_ascii_case(&'r') || c.is_whitespace());
            s.parse::<f32>().ok().map(|v| v / 50.0)
        } else {
            None
        }
    }
}

/// Formats a `−1..=1` panning value as `"NNL"`, `"C"`, or `"NNR"`.
pub const PAN: FloatFormatter = (pan_to_string, pan_from_string);

// -------------------------------------------------------------------------------------------------

fn ratio_to_string(v: f32) -> String {
    if v >= 20.0 {
        "LIMIT".to_string()
    } else {
        format!("1:{:.2}", v)
    }
}

fn ratio_from_string(s: &str) -> Option<f32> {
    let trimmed = s.trim();
    if trimmed.eq_ignore_ascii_case("LIMIT") {
        Some(20.0)
    } else if let Some(ratio_str) = trimmed.strip_prefix("1:") {
        ratio_str.parse::<f32>().ok()
    } else {
        trimmed.parse::<f32>().ok()
    }
}

/// Formats a compression ratio as `"1:N.NN"` or `"LIMIT"`.
pub const RATIO: FloatFormatter = (ratio_to_string, ratio_from_string);

// -------------------------------------------------------------------------------------------------

fn degrees_to_string(v: f32) -> String {
    format!("{}°", v.to_degrees().round() as i32)
}

fn degrees_from_string(s: &str) -> Option<f32> {
    s.trim()
        .trim_end_matches('°')
        .parse::<f32>()
        .map(|f| f.to_radians())
        .ok()
}

/// Formats a radian value as integer degrees (e.g. `"90°"`).
pub const DEGREES: FloatFormatter = (degrees_to_string, degrees_from_string);
