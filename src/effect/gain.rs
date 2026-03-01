use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{
        formatters, EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        buffer::{scale_buffer, InterleavedBufferMut},
        db_to_linear,
        dsp::filters::dc::{DcFilter, DcFilterMode},
    },
    Error, Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// DC filter mode options for the [`GainEffect`].
#[derive(
    Debug, Default, Copy, Clone, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
pub enum GainEffectDcFilterMode {
    /// No DC filtering.
    #[default]
    Off,
    /// ~1Hz cutoff: very gentle, might not remove all DC offset.
    Slow,
    /// ~5Hz cutoff: good for most cases.
    Default,
    /// ~20Hz cutoff: aggressive, might affect very low frequencies.
    Fast,
}

impl GainEffectDcFilterMode {
    /// Returns the corresponding DSP filter mode, or `None` when `Off`.
    fn to_dc_mode(self) -> Option<DcFilterMode> {
        match self {
            GainEffectDcFilterMode::Off => None,
            GainEffectDcFilterMode::Slow => Some(DcFilterMode::Slow),
            GainEffectDcFilterMode::Default => Some(DcFilterMode::Default),
            GainEffectDcFilterMode::Fast => Some(DcFilterMode::Fast),
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Multi-channel gain effect with optional DC filter.
pub struct GainEffect {
    // Parameters
    gain: SmoothedParameterValue,
    dc_filter_mode: EnumParameterValue<GainEffectDcFilterMode>,
    // Internal state
    dc_filters: Vec<DcFilter>,
    sample_rate: u32,
    channel_count: usize,
}

impl GainEffect {
    pub const EFFECT_NAME: &str = "Gain";

    const MIN_DB: f32 = -60.0;
    const MAX_DB: f32 = 24.0;

    pub const GAIN: FloatParameter = FloatParameter::new(
        FourCC(*b"gain"),
        "Gain",
        0.000001..=15.848932, // Self::MIN_DB..=Self::MAX_DB,
        1.0,                  // 0dB
    )
    .with_scaling(ParameterScaling::Decibel(Self::MIN_DB, Self::MAX_DB))
    .with_formatter(formatters::GAIN);

    pub const DC_FILTER: EnumParameter = EnumParameter::new(
        FourCC(*b"dcfm"),
        "DC Filter",
        GainEffectDcFilterMode::VARIANTS,
        GainEffectDcFilterMode::Off as usize,
    );

    /// Creates a new `GainEffect` with default gain (0dB = unity gain).
    pub fn new() -> Self {
        Self {
            sample_rate: 0,
            channel_count: 0,
            gain: SmoothedParameterValue::from_description(Self::GAIN),
            dc_filter_mode: EnumParameterValue::from_description(Self::DC_FILTER),
            dc_filters: Vec::new(),
        }
    }

    /// Creates a new `GainEffect` with the given parameters.
    pub fn with_parameters(gain_db: f32, dc_mode: GainEffectDcFilterMode) -> Self {
        let mut effect = Self::new();
        let gain_linear = db_to_linear(gain_db.clamp(Self::MIN_DB, Self::MAX_DB));
        effect.gain.init_value(gain_linear);
        effect.dc_filter_mode.set_value(dc_mode);
        effect
    }
}

impl Default for GainEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for GainEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        1
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![self.gain.description(), self.dc_filter_mode.description()]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;
        self.gain.set_sample_rate(sample_rate);
        let dc_mode = self
            .dc_filter_mode
            .value()
            .to_dc_mode()
            .unwrap_or(DcFilterMode::Default);
        self.dc_filters = (0..channel_count)
            .map(|_| DcFilter::new(sample_rate, dc_mode))
            .collect();
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        // DC filter
        if self.dc_filter_mode.value() != GainEffectDcFilterMode::Off {
            for (filter, channel_iter) in self
                .dc_filters
                .iter_mut()
                .zip(output.channels_mut(self.channel_count))
            {
                filter.process(channel_iter);
            }
        }
        // Gain
        if self.gain.value_need_ramp() {
            for frame in output.frames_mut(self.channel_count) {
                let gain = self.gain.next_value();
                for sample in frame {
                    *sample *= gain;
                }
            }
        } else {
            let gain = self.gain.target_value();
            scale_buffer(output, gain);
        }
    }

    fn process_tail(&self) -> Option<usize> {
        if let Some(dc_mode) = self.dc_filter_mode.value().to_dc_mode() {
            // One-pole DC filter has a state that decays
            Some(self.sample_rate as usize / dc_mode.hz() as usize)
        } else {
            Some(0)
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::GAIN.id() => {
                self.gain.apply_update(value);
                Ok(())
            }
            _ if id == Self::DC_FILTER.id() => {
                self.dc_filter_mode.apply_update(value);
                if let Some(dsp_mode) = self.dc_filter_mode.value().to_dc_mode() {
                    for filter in &mut self.dc_filters {
                        filter.set_mode(dsp_mode, self.sample_rate);
                    }
                } else {
                    for filter in &mut self.dc_filters {
                        filter.reset();
                    }
                }
                Ok(())
            }
            _ => Err(Error::ParameterError(format!(
                "Unknown parameter: '{id}' for effect '{}'",
                self.name()
            ))),
        }
    }
}
