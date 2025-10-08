use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{ClonableParameter, EnumParameter, EnumParameterValue, ParameterValueUpdate},
    utils::{
        filter::dc::{DcFilter, DcFilterMode},
        InterleavedBufferMut,
    },
    Error,
};

// -------------------------------------------------------------------------------------------------

pub type DcFilterEffectMode = DcFilterMode;

// -------------------------------------------------------------------------------------------------

/// A filter effect that applies a DC-blocking filter to an audio buffer.
#[derive(Clone)]
pub struct DcFilterEffect {
    channel_count: usize,
    sample_rate: u32,
    filters: Vec<DcFilter>,
    mode: EnumParameterValue<DcFilterEffectMode>,
}

impl DcFilterEffect {
    pub const EFFECT_NAME: &str = "DcFilterEffect";
    pub const MODE_ID: FourCC = FourCC(*b"mode");

    /// Creates a new `DcFilterEffect` with default parameter values.
    pub fn new() -> Self {
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: Vec::new(),
            mode: EnumParameterValue::from_description(EnumParameter::new(
                Self::MODE_ID,
                "Mode",
                DcFilterEffectMode::Default,
            )),
        }
    }

    /// Creates a new `DcFilterEffect` with the given parameter values.
    pub fn with_parameters(mode: DcFilterEffectMode) -> Self {
        let mut filter = Self::default();
        filter.mode.set_value(mode);
        filter
    }
}

impl Default for DcFilterEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for DcFilterEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn parameters(&self) -> Vec<&dyn ClonableParameter> {
        vec![self.mode.description()]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;

        self.filters.clear();
        for _ in 0..channel_count {
            let mut filter = DcFilter::new();
            filter.init(sample_rate, self.mode.value());
            self.filters.push(filter);
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
            Self::MODE_ID => self.mode.apply_update(value),
            _ => return Err(Error::ParameterError(format!("Unknown parameter: {id}"))),
        }
        for filter in &mut self.filters {
            filter.init(self.sample_rate, self.mode.value());
        }
        Ok(())
    }
}
