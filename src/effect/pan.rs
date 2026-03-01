use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        formatters, BooleanParameter, BooleanParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{buffer::InterleavedBufferMut, panning_factors},
    Error, Parameter,
};

// -------------------------------------------------------------------------------------------------

/// Stereo panning effect with pan, stereo width and phase invert controls.
///
/// Processing order per frame: phase invert → width (mid/side) → pan (constant power).
pub struct PanningEffect {
    channel_count: usize,
    // Parameters
    pan: SmoothedParameterValue,
    width: SmoothedParameterValue,
    invert_l: BooleanParameterValue,
    invert_r: BooleanParameterValue,
}

impl PanningEffect {
    pub const EFFECT_NAME: &str = "Panning";

    pub const PAN: FloatParameter = FloatParameter::new(
        FourCC(*b"pan "),
        "Pan",
        -1.0..=1.0,
        0.0, // center
    )
    .with_formatter(formatters::PAN);

    pub const WIDTH: FloatParameter = FloatParameter::new(
        FourCC(*b"wdth"),
        "Width",
        0.0..=2.0,
        1.0, // normal stereo
    )
    .with_formatter(formatters::PERCENT);

    pub const INVERT_L: BooleanParameter =
        BooleanParameter::new(FourCC(*b"invl"), "Invert L", false);

    pub const INVERT_R: BooleanParameter =
        BooleanParameter::new(FourCC(*b"invr"), "Invert R", false);

    /// Creates a new `PanningEffect` with default parameters (center, unity width, no invert).
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            pan: SmoothedParameterValue::from_description(Self::PAN),
            width: SmoothedParameterValue::from_description(Self::WIDTH),
            invert_l: BooleanParameterValue::from_description(Self::INVERT_L),
            invert_r: BooleanParameterValue::from_description(Self::INVERT_R),
        }
    }
}

impl Default for PanningEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for PanningEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        1
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![
            self.pan.description(),
            self.width.description(),
            self.invert_l.description(),
            self.invert_r.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "PanningEffect only supports stereo I/O".to_string(),
            ));
        }
        self.channel_count = channel_count;
        self.pan.set_sample_rate(sample_rate);
        self.width.set_sample_rate(sample_rate);
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        debug_assert!(self.channel_count == 2);

        let invert_l = if self.invert_l.value() { -1.0f32 } else { 1.0 };
        let invert_r = if self.invert_r.value() { -1.0f32 } else { 1.0 };
        let has_invert = invert_l < 0.0 || invert_r < 0.0;

        let pan_ramping = self.pan.value_need_ramp();
        let width_ramping = self.width.value_need_ramp();

        // Fast path: nothing to do
        if !has_invert
            && !pan_ramping
            && !width_ramping
            && self.pan.target_value().abs() < 1e-6
            && (self.width.target_value() - 1.0).abs() < 1e-6
        {
            return;
        }

        for frame in output.as_frames_mut::<2>() {
            // 1. Phase invert
            let mut l = frame[0] * invert_l;
            let mut r = frame[1] * invert_r;

            // 2. Width via mid/side
            let width = if width_ramping {
                self.width.next_value()
            } else {
                self.width.target_value()
            };
            if (width - 1.0).abs() > 1e-6 {
                let mid = (l + r) * 0.5;
                let side = (l - r) * 0.5;
                l = mid + side * width;
                r = mid - side * width;
            }

            // 3. Pan (constant power)
            let pan = if pan_ramping {
                self.pan.next_value()
            } else {
                self.pan.target_value()
            };
            if pan.abs() > 1e-6 {
                let (pan_l, pan_r) = panning_factors(pan);
                l *= pan_l;
                r *= pan_r;
            }

            frame[0] = l;
            frame[1] = r;
        }
    }

    fn process_tail(&self) -> Option<usize> {
        Some(0)
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::PAN.id() => {
                self.pan.apply_update(value);
                Ok(())
            }
            _ if id == Self::WIDTH.id() => {
                self.width.apply_update(value);
                Ok(())
            }
            _ if id == Self::INVERT_L.id() => {
                self.invert_l.apply_update(value);
                Ok(())
            }
            _ if id == Self::INVERT_R.id() => {
                self.invert_r.apply_update(value);
                Ok(())
            }
            _ => Err(Error::ParameterError(format!(
                "Unknown parameter: '{id}' for effect '{}'",
                self.name()
            ))),
        }
    }
}
