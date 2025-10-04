use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, FloatParameterValue, Parameter,
        ParameterValueUpdate,
    },
    utils::{
        filter::svf::{SvfFilter, SvfFilterType},
        InterleavedBufferMut,
    },
    Error,
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
    filter_type: EnumParameterValue<FilterEffectType>,
    cutoff: FloatParameterValue,
    q: FloatParameterValue,
    gain: FloatParameterValue,
}

impl FilterEffect {
    pub const EFFECT_NAME: &str = "FilterEffect";
    pub const TYPE_ID: FourCC = FourCC(*b"type");
    pub const CUTOFF_ID: FourCC = FourCC(*b"cuto");
    pub const Q_ID: FourCC = FourCC(*b"q   ");
    pub const GAIN_ID: FourCC = FourCC(*b"gain");

    /// Creates a new `FilterEffect` with default parameter values.
    pub fn new() -> Self {
        let filter_type = SvfFilterType::Lowpass;
        let cutoff = 22050.0;
        let q = 0.707;
        let gain = 0.0;
        let template_filter = SvfFilter::new(filter_type, 44100, cutoff, q, gain).unwrap();
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: vec![template_filter],
            filter_type: EnumParameter::new(Self::TYPE_ID, "Type", FilterEffectType::Lowpass)
                .into(),
            cutoff: FloatParameter::new(Self::CUTOFF_ID, "Cutoff", 20.0..=22050.0, 22050.0).into(),
            q: FloatParameter::new(Self::Q_ID, "Q", 0.001..=24.0, 0.707).into(),
            gain: FloatParameter::new(Self::GAIN_ID, "Gain", -24.0..=24.0, 0.0).into(),
        }
    }

    /// Creates a new `DistortionEffect` with the given parameter values.
    pub fn with_parameters(filter_type: SvfFilterType, cutoff: f32, q: f32, gain: f32) -> Self {
        let mut filter = Self::new();
        filter.filter_type.set_value(filter_type);
        filter.cutoff.set_value(cutoff);
        filter.q.set_value(q);
        filter.gain.set_value(gain);
        filter.filters[0]
            .set(filter_type, 44100, cutoff, q, gain)
            .unwrap();
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

    fn parameters(&self) -> Vec<Box<dyn Parameter>> {
        vec![
            Box::new(self.filter_type.description().clone()),
            Box::new(self.cutoff.description().clone()),
            Box::new(self.q.description().clone()),
            Box::new(self.gain.description().clone()),
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

        let template = self.filters.first().ok_or_else(|| {
            Error::ParameterError(
                "FilterEffect must be created with `with_parameters` or `default`".to_string(),
            )
        })?;

        let filter_type = template.coefficients().filter_type();
        let cutoff = template.coefficients().cutoff();
        let q = template.coefficients().q();
        let gain = template.coefficients().gain();

        self.filters.clear();
        for _channel in 0..channel_count {
            self.filters.push(SvfFilter::new(
                filter_type,
                sample_rate,
                cutoff.clamp(20.0, sample_rate as f32 / 2.0),
                q,
                gain,
            )?);
        }
        Ok(())
    }

    fn process(&mut self, mut buffer: &mut [f32], _time: &EffectTime) {
        // Apply filter to each channel
        for (filter, channel_iter) in self
            .filters
            .iter_mut()
            .zip(buffer.channels_mut(self.channel_count))
        {
            filter.process(channel_iter);
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

        for filter in &mut self.filters {
            let coeffs = filter.coefficients_mut();
            let result = match id {
                Self::TYPE_ID => coeffs.set_filter_type(*self.filter_type.value()),
                Self::CUTOFF_ID => coeffs.set_cutoff(*self.cutoff.value()),
                Self::Q_ID => coeffs.set_q(*self.q.value()),
                Self::GAIN_ID => coeffs.set_gain(*self.gain.value()),
                _ => Ok(()),
            };
            if let Err(err) = result {
                log::error!("Failed to apply new filter parameters: {err}");
            }
        }
        Ok(())
    }
}
