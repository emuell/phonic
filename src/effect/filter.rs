use four_cc::FourCC;
use strum::{Display, EnumIter, EnumString};

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        buffer::InterleavedBufferMut,
        dsp::filters::biquad::{BiquadFilter, BiquadFilterCoefficients, BiquadFilterType},
        smoothing::LinearSmoothedValue,
    },
    ClonableParameter, Error, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// Filter type used in `FilterEffect`.
#[derive(Default, Clone, Copy, PartialEq, Display, EnumIter, EnumString)]
#[allow(unused)]
pub enum FilterEffectType {
    #[default]
    Lowpass,
    Bandpass,
    Bandstop,
    Highpass,
}

impl From<FilterEffectType> for BiquadFilterType {
    fn from(val: FilterEffectType) -> Self {
        match val {
            FilterEffectType::Lowpass => BiquadFilterType::Lowpass,
            FilterEffectType::Bandpass => BiquadFilterType::Bandpass,
            FilterEffectType::Bandstop => BiquadFilterType::Notch,
            FilterEffectType::Highpass => BiquadFilterType::Highpass,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Multi-channel filter effect tht applies an SVF biquad filter with configurable filter types.
#[derive(Clone)]
pub struct FilterEffect {
    channel_count: usize,
    sample_rate: u32,
    filters: Vec<BiquadFilter>,
    filter_coefficients: BiquadFilterCoefficients,
    filter_type: EnumParameterValue<FilterEffectType>,
    cutoff: SmoothedParameterValue,
    q: SmoothedParameterValue<LinearSmoothedValue>,
}

impl FilterEffect {
    pub const EFFECT_NAME: &str = "FilterEffect";
    pub const TYPE_ID: FourCC = FourCC(*b"type");
    pub const CUTOFF_ID: FourCC = FourCC(*b"cuto");
    pub const Q_ID: FourCC = FourCC(*b"fltq");

    /// Creates a new `FilterEffect` with default parameter values.
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: vec![],
            filter_coefficients: BiquadFilterCoefficients::new(
                BiquadFilterType::Lowpass,
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
                    20.0..=20000.0,
                    20000.0, //
                )
                .with_unit("Hz")
                .with_scaling(ParameterScaling::Exponential(2.5)),
            ),
            q: SmoothedParameterValue::from_description(FloatParameter::new(
                Self::Q_ID,
                "Resonance",
                0.001..=4.0,
                0.707, //
            )),
        }
    }

    /// Creates a new `DistortionEffect` with the given parameter values.
    pub fn with_parameters(filter_type: FilterEffectType, cutoff: f32, q: f32) -> Self {
        let mut filter = Self::new();
        filter.filter_type.set_value(filter_type);
        filter.cutoff.init_value(cutoff);
        filter.q.init_value(q);
        filter
            .filter_coefficients
            .set(filter_type.into(), 44100, cutoff, q, 0.0)
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
        self.filters.resize_with(channel_count, BiquadFilter::new);

        self.cutoff.set_sample_rate(sample_rate);
        self.q.set_sample_rate(sample_rate);

        Ok(())
    }

    fn process(&mut self, mut buffer: &mut [f32], _time: &EffectTime) {
        // Apply filter with parameter ramping
        if self.cutoff.value_need_ramp() || self.q.value_need_ramp() {
            for frame in buffer.frames_mut(self.channel_count) {
                let cutoff = self
                    .cutoff
                    .next_value()
                    .clamp(20.0, self.sample_rate as f32 / 2.0);
                let q = self.q.next_value();
                self.filter_coefficients
                    .set(
                        self.filter_type.value().into(),
                        self.sample_rate,
                        cutoff,
                        q,
                        0.0,
                    )
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

    fn process_tail(&self) -> Option<usize> {
        // Biquad filter has internal state that rings out. 100ms (sample_rate/10) covers
        // the decay time for most filter configurations.
        Some(self.sample_rate as usize / 10)
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
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };

        let result = match id {
            Self::TYPE_ID => self
                .filter_coefficients
                .set_filter_type(self.filter_type.value().into()),
            _ => Ok(()),
        };

        if let Err(err) = result {
            log::error!("Failed to apply new filter parameters: {err}");
        }
        Ok(())
    }
}
