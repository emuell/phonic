use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    utils::{
        filter::dc::{DcFilter, DcFilterMode},
        InterleavedBufferMut,
    },
    Error,
};
use std::any::Any;

// -------------------------------------------------------------------------------------------------

/// Message type for `DcFilterEffect` to change filter parameters.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum DcFilterEffectMessage {
    // Set the DC filter mode (frequency)
    SetMode(DcFilterMode),
}

impl EffectMessage for DcFilterEffectMessage {
    fn effect_name(&self) -> &'static str {
        DcFilterEffect::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A filter effect that applies a DC-blocking filter to an audio buffer.
#[derive(Clone)]
pub struct DcFilterEffect {
    channel_count: usize,
    sample_rate: u32,
    filters: Vec<DcFilter>,
    mode: DcFilterMode,
}

impl DcFilterEffect {
    const DEFAULT_MODE: DcFilterMode = DcFilterMode::Default;

    pub fn with_parameters(mode: DcFilterMode) -> Self {
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: Vec::new(),
            mode,
        }
    }
}

impl Default for DcFilterEffect {
    fn default() -> Self {
        Self::with_parameters(Self::DEFAULT_MODE)
    }
}

impl Effect for DcFilterEffect {
    fn name() -> &'static str {
        "DcFilterEffect"
    }

    /// Sends a message to update the filter parameters.
    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<DcFilterEffectMessage>() {
            match message {
                DcFilterEffectMessage::SetMode(mode) => {
                    self.mode = *mode;
                    for filter in &mut self.filters {
                        filter.init(self.sample_rate, *mode);
                    }
                }
            }
        } else {
            log::error!("DcFilterEffect: Invalid/unknown message payload");
        }
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
            filter.init(sample_rate, self.mode);
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
}
