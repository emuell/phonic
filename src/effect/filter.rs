use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        filter::svf::{SvfFilter, SvfFilterCoefficients, SvfFilterType},
        InterleavedBufferMut,
    },
    ClonableParameter, Error,
};

// -------------------------------------------------------------------------------------------------

/// Filter type used in `FilterEffect`.
pub type FilterEffectType = SvfFilterType;

// -------------------------------------------------------------------------------------------------

/// A filter effect that applies an SVF biquad filter to an audio buffer.
#[derive(Clone)]
pub struct FilterEffect {
    channel_count: usize,
    sample_rate: u32,
    filters: Vec<SvfFilter>,
    filter_coefficients: SvfFilterCoefficients,
    filter_type: EnumParameterValue<FilterEffectType>,
    cutoff: SmoothedParameterValue,
    q: SmoothedParameterValue,
    gain: SmoothedParameterValue,
}

impl FilterEffect {
    pub const EFFECT_NAME: &str = "FilterEffect";
    pub const TYPE_ID: FourCC = FourCC(*b"type");
    pub const CUTOFF_ID: FourCC = FourCC(*b"cuto");
    pub const Q_ID: FourCC = FourCC(*b"q   ");
    pub const GAIN_ID: FourCC = FourCC(*b"gain");

    /// Creates a new `FilterEffect` with default parameter values.
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: vec![],
            filter_coefficients: SvfFilterCoefficients::new(
                SvfFilterType::Lowpass,
                44100,
                22050.0,
                0.707,
                0.0,
            )
            .expect("Failed to create default filter"),
            filter_type: EnumParameterValue::from_description(EnumParameter::new(
                Self::TYPE_ID,
                "Type",
                FilterEffectType::Lowpass,
            )),
            cutoff: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::CUTOFF_ID,
                    "Cutoff",
                    20.0..=22050.0,
                    22050.0, //
                )
                .with_unit("Hz"),
            ),
            q: SmoothedParameterValue::from_description(FloatParameter::new(
                Self::Q_ID,
                "Q",
                0.001..=24.0,
                0.707, //
            )),
            gain: SmoothedParameterValue::from_description(
                FloatParameter::new(
                    Self::GAIN_ID,
                    "Gain",
                    -24.0..=24.0,
                    0.0, //
                )
                .with_unit("dB"),
            ),
        }
    }

    /// Creates a new `DistortionEffect` with the given parameter values.
    pub fn with_parameters(filter_type: SvfFilterType, cutoff: f32, q: f32, gain: f32) -> Self {
        let mut filter = Self::new();
        filter.filter_type.set_value(filter_type);
        filter.cutoff.init_value(cutoff);
        filter.q.init_value(q);
        filter.gain.init_value(gain);
        filter
            .filter_coefficients
            .set(filter_type, 44100, cutoff, q, gain)
            .expect("Invalid filter parameters");
        filter
    }
}

impl Default for FilterEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for FilterEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![
            self.filter_type.description(),
            self.cutoff.description(),
            self.q.description(),
            self.gain.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;

        // make sure cutoff is still valid
        self.filter_coefficients
            .set_cutoff(
                self.filter_coefficients
                    .cutoff()
                    .clamp(20.0, sample_rate as f32 / 2.0),
            )
            .expect("Failed to set filter parameters");
        self.filters.resize_with(channel_count, SvfFilter::new);

        self.cutoff.set_sample_rate(sample_rate);
        self.q.set_sample_rate(sample_rate);
        self.gain.set_sample_rate(sample_rate);

        Ok(())
    }

    fn process(&mut self, mut buffer: &mut [f32], _time: &EffectTime) {
        // Apply filter with parameter ramping
        if self.cutoff.value_need_ramp() || self.q.value_need_ramp() || self.gain.value_need_ramp()
        {
            for frame in buffer.frames_mut(self.channel_count) {
                let cutoff = self
                    .cutoff
                    .next_value()
                    .clamp(20.0, self.sample_rate as f32 / 2.0);
                let q = self.q.next_value();
                let gain = self.gain.next_value();
                self.filter_coefficients
                    .set(self.filter_type.value(), self.sample_rate, cutoff, q, gain)
                    .expect("Invalid filter parameters");
                for (sample, filter) in frame.zip(self.filters.iter_mut()) {
                    *sample = filter.process_sample(
                        &self.filter_coefficients,
                        *sample as f64, //
                    ) as f32;
                }
            }
        } else {
            // Apply filter without parameter ramping
            for (filter, channel_iter) in self
                .filters
                .iter_mut()
                .zip(buffer.channels_mut(self.channel_count))
            {
                filter.process(&self.filter_coefficients, channel_iter);
            }
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            Self::TYPE_ID => self.filter_type.apply_update(value),
            Self::CUTOFF_ID => self.cutoff.apply_update(value),
            Self::Q_ID => self.q.apply_update(value),
            Self::GAIN_ID => self.gain.apply_update(value),
            _ => return Err(Error::ParameterError(format!("Unknown parameter: {id}"))),
        };

        let result = match id {
            Self::TYPE_ID => self
                .filter_coefficients
                .set_filter_type(self.filter_type.value()),
            _ => Ok(()),
        };

        if let Err(err) = result {
            log::error!("Failed to apply new filter parameters: {err}");
        }
        Ok(())
    }
}
