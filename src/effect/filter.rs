use std::any::Any;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    utils::{
        filter::svf::{SvfFilter, SvfFilterType},
        InterleavedBufferMut,
    },
    Error,
};

// -------------------------------------------------------------------------------------------------

/// Filter type parameter
pub type FilterEffectType = SvfFilterType;

// -------------------------------------------------------------------------------------------------

/// Message type for `FilterEffect` to change filter parameters.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum FilterEffectMessage {
    /// Set all filter parameters at once.
    Init(
        FilterEffectType, // Type
        f32,              // Cutoff
        f32,              // Q
        f32,              // Gain
    ),
    /// Change the filter type.
    SetFilterType(FilterEffectType),
    /// Change the cutoff frequency in Hz. Range: 20.0 to sample_rate / 2.
    SetCutoff(f32),
    /// Change the resonance (Q factor). Range: > 0.0.
    SetQ(f32),
    /// Change the gain parameter in dB (applied for shelving/peak filters only).
    SetGain(f32),
}

impl EffectMessage for FilterEffectMessage {
    fn effect_name(&self) -> &'static str {
        FilterEffect::name()
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// A filter effect that applies an SVF biquad filter to an audio buffer.
#[derive(Clone)]
pub struct FilterEffect {
    channel_count: usize,
    sample_rate: u32,
    filters: Vec<SvfFilter>,
}

impl FilterEffect {
    const DEFAULT_FILTER_TYPE: SvfFilterType = SvfFilterType::Lowpass;
    const DEFAULT_CUTOFF: f32 = 22050.0; // This will be clamped to nyquist in initialize
    const DEFAULT_Q: f32 = 0.707;
    const DEFAULT_GAIN: f32 = 0.0;

    pub fn with_parameters(filter_type: SvfFilterType, cutoff: f32, q: f32, gain: f32) -> Self {
        let template_filter =
            SvfFilter::new(filter_type, 44100, cutoff.max(20.0), q.max(0.001), gain).unwrap();
        Self {
            channel_count: 0,
            sample_rate: 0,
            filters: vec![template_filter],
        }
    }
}

impl Default for FilterEffect {
    fn default() -> Self {
        Self::with_parameters(
            Self::DEFAULT_FILTER_TYPE,
            Self::DEFAULT_CUTOFF,
            Self::DEFAULT_Q,
            Self::DEFAULT_GAIN,
        )
    }
}

impl Effect for FilterEffect {
    fn name() -> &'static str {
        "FilterEffect"
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

    fn process_message(&mut self, message: &EffectMessagePayload) {
        if let Some(message) = message.payload().downcast_ref::<FilterEffectMessage>() {
            let nyquist = self.sample_rate as f32 / 2.0;
            const MIN_Q: f32 = 0.001;
            const MIN_CUTOFF: f32 = 20.0;

            for filter in &mut self.filters {
                let coeffs = filter.coefficients_mut();
                let result = match message {
                    FilterEffectMessage::Init(ft, c, q, g) => {
                        let cutoff = c.clamp(MIN_CUTOFF, nyquist);
                        let q_clamped = q.max(MIN_Q);
                        coeffs.set(*ft, self.sample_rate, cutoff, q_clamped, *g)
                    }
                    FilterEffectMessage::SetFilterType(ft) => coeffs.set_filter_type(*ft),
                    FilterEffectMessage::SetCutoff(c) => {
                        let cutoff = c.clamp(MIN_CUTOFF, nyquist);
                        coeffs.set_cutoff(cutoff)
                    }
                    FilterEffectMessage::SetQ(q) => {
                        let q_clamped = q.max(MIN_Q);
                        coeffs.set_q(q_clamped)
                    }
                    FilterEffectMessage::SetGain(g) => coeffs.set_gain(*g),
                };
                if let Err(err) = result {
                    log::error!("Failed to apply new filter parameters: {err}");
                }
            }
        } else {
            log::error!("FilterEffect: Invalid/unknown message payload");
        }
    }
}
