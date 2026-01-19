use four_cc::FourCC;

use crate::{
    effect::{Effect, EffectTime},
    parameter::{FloatParameter, ParameterValueUpdate, SmoothedParameterValue},
    utils::{
        dsp::filters::biquad::{BiquadFilter, BiquadFilterCoefficients, BiquadFilterType},
        smoothing::LinearSmoothedValue,
    },
    Error, Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// Multi-channel 5-band parametric equalizer.
///
/// First band is using a low-shelf filter, last band a high shelf filter, the other bands are
/// notch filters with configurable band width.
pub struct Eq5Effect {
    sample_rate: u32,
    channel_count: usize,

    // Parameters (arrays of 5)
    gains: [SmoothedParameterValue; 5],
    frequencies: [SmoothedParameterValue; 5],
    bandwidths: [SmoothedParameterValue<LinearSmoothedValue>; 5],

    // Runtime data
    // 5 coefficient sets (one per band)
    filter_coeffs: [BiquadFilterCoefficients; 5],
    // filters per channel: channel_count * 5 filters
    filters: Vec<[BiquadFilter; 5]>,
}

impl Eq5Effect {
    pub const EFFECT_NAME: &str = "Eq5";

    pub const GAINS: [FloatParameter; 5] = [
        FloatParameter::new(
            FourCC(*b"gan1"), //
            "Gain 1",
            -20.0..=20.0,
            0.0,
        )
        .with_unit("dB"),
        FloatParameter::new(
            FourCC(*b"gan2"), //
            "Gain 2",
            -20.0..=20.0,
            0.0,
        )
        .with_unit("dB"),
        FloatParameter::new(
            FourCC(*b"gan3"), //
            "Gain 3",
            -20.0..=20.0,
            0.0,
        )
        .with_unit("dB"),
        FloatParameter::new(
            FourCC(*b"gan4"), //
            "Gain 4",
            -20.0..=20.0,
            0.0,
        )
        .with_unit("dB"),
        FloatParameter::new(
            FourCC(*b"gan5"), //
            "Gain 5",
            -20.0..=20.0,
            0.0,
        )
        .with_unit("dB"),
    ];

    pub const FREQUENCIES: [FloatParameter; 5] = [
        FloatParameter::new(
            FourCC(*b"frq1"), //
            "Frequency 1",
            20.0..=20000.0,
            100.0,
        )
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.5)),
        FloatParameter::new(
            FourCC(*b"frq2"), //
            "Frequency 2",
            20.0..=20000.0,
            1000.0,
        )
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.5)),
        FloatParameter::new(
            FourCC(*b"frq3"), //
            "Frequency 3",
            20.0..=20000.0,
            4000.0,
        )
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.5)),
        FloatParameter::new(
            FourCC(*b"frq4"), //
            "Frequency 4",
            20.0..=20000.0,
            8000.0,
        )
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.5)),
        FloatParameter::new(
            FourCC(*b"frq5"), //
            "Frequency 5",
            20.0..=20000.0,
            12000.0,
        )
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.5)),
    ];

    pub const BANDWIDTHS: [FloatParameter; 5] = [
        FloatParameter::new(
            FourCC(*b"bw_1"), // Lowshelf
            "Bandwidth 1",
            0.0001..=1.0,
            1.0,
        ),
        FloatParameter::new(
            FourCC(*b"bw_2"), //
            "Bandwidth 2",
            0.0001..=4.0,
            4.0,
        ),
        FloatParameter::new(
            FourCC(*b"bw_3"), //
            "Bandwidth 3",
            0.0001..=4.0,
            4.0,
        ),
        FloatParameter::new(
            FourCC(*b"bw_4"), //
            "Bandwidth 4",
            0.0001..=4.0,
            4.0,
        ),
        FloatParameter::new(
            FourCC(*b"bw_5"), // Highshelf
            "Bandwidth 5",
            0.0001..=1.0,
            1.0,
        ),
    ];

    /// Creates a new `Eq5Effect` with default parameter values.
    pub fn new() -> Self {
        Self {
            sample_rate: 0,
            channel_count: 0,
            gains: Self::GAINS.map(
                SmoothedParameterValue::from_description, //
            ),
            frequencies: Self::FREQUENCIES.map(
                SmoothedParameterValue::from_description, //
            ),
            bandwidths: Self::BANDWIDTHS.map(|p| {
                SmoothedParameterValue::from_description(p)
                    .with_smoother(LinearSmoothedValue::default())
            }),
            filter_coeffs: Default::default(),
            filters: Vec::new(),
        }
    }

    // Update filter coefficients on parameter changes.
    fn update_filter_coefficients(&mut self) -> Result<(), Error> {
        for i in 0..5 {
            let filter_type = match i {
                0 => BiquadFilterType::Lowshelf,
                4 => BiquadFilterType::Highshelf,
                _ => BiquadFilterType::Bell,
            };
            let cutoff = self.frequencies[i].current_value();
            let q = self.bandwidths[i].current_value();
            let gain = self.gains[i].current_value();
            self.filter_coeffs[i].set(filter_type, self.sample_rate, cutoff, q, gain)?;
        }
        Ok(())
    }

    // Ramp filter coefficients on parameter changes.
    fn ramp_filter_coefficients(&mut self) -> Result<(), Error> {
        for i in 0..5 {
            let filter_type = match i {
                0 => BiquadFilterType::Lowshelf,
                4 => BiquadFilterType::Highshelf,
                _ => BiquadFilterType::Bell,
            };
            let q = match i {
                0 | 4 => self.bandwidths[i].next_value(),
                _ => self.bandwidths[i].next_value().max(0.001).recip(),
            };
            let cutoff = self.frequencies[i].next_value();
            let gain = self.gains[i].next_value();
            self.filter_coeffs[i].set(filter_type, self.sample_rate, cutoff, q, gain)?;
        }
        Ok(())
    }

    /// Reset filters and parameter smoothing.
    fn reset(&mut self) {
        for channel_filters in &mut self.filters {
            for filter in channel_filters {
                filter.reset();
            }
        }

        for gain in &mut self.gains {
            gain.init_value(gain.target_value());
        }
        for freq in &mut self.frequencies {
            freq.init_value(freq.target_value());
        }
        for bw in &mut self.bandwidths {
            bw.init_value(bw.target_value());
        }
    }
}

impl Default for Eq5Effect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for Eq5Effect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        3
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![
            self.gains[0].description(),
            self.frequencies[0].description(),
            self.bandwidths[0].description(),
            self.gains[1].description(),
            self.frequencies[1].description(),
            self.bandwidths[1].description(),
            self.gains[2].description(),
            self.frequencies[2].description(),
            self.bandwidths[2].description(),
            self.gains[3].description(),
            self.frequencies[3].description(),
            self.bandwidths[3].description(),
            self.gains[4].description(),
            self.frequencies[4].description(),
            self.bandwidths[4].description(),
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

        // Set sample rates on all smoothed parameters
        for gain in &mut self.gains {
            gain.set_sample_rate(sample_rate);
        }
        for freq in &mut self.frequencies {
            freq.set_sample_rate(sample_rate);
        }
        for bw in &mut self.bandwidths {
            bw.set_sample_rate(sample_rate);
        }

        // Initialize all filter coefficients with default values
        self.update_filter_coefficients()?;

        // Allocate filters for each channel
        self.filters = vec![Default::default(); channel_count];

        self.reset();

        Ok(())
    }

    fn process(&mut self, output: &mut [f32], _time: &EffectTime) {
        let need_ramp = self.frequencies.iter().any(|f| f.value_need_ramp())
            || self.bandwidths.iter().any(|b| b.value_need_ramp())
            || self.gains.iter().any(|g| g.value_need_ramp());

        // Process each frame
        let frame_count = output.len() / self.channel_count;
        for frame_idx in 0..frame_count {
            // Update filter coefficients if parameters changed
            if need_ramp {
                self.ramp_filter_coefficients()
                    .expect("Failed to update filter coefficients");
            }

            // Process each channel in this frame
            for ch in 0..self.channel_count {
                let sample_idx = frame_idx * self.channel_count + ch;
                let mut sample = output[sample_idx];

                // Chain all 5 filters for this channel
                for i in 0..5 {
                    sample = self.filters[ch][i]
                        .process_sample(&self.filter_coeffs[i], sample as f64)
                        as f32;
                }

                output[sample_idx] = sample;
            }
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // Five cascaded biquad filters ring longer than a single filter.
        // 200ms (sample_rate/5) is a safe estimate for the combined decay.
        Some(self.sample_rate as usize / 5)
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::GAINS[0].id() => self.gains[0].apply_update(value),
            _ if id == Self::GAINS[1].id() => self.gains[1].apply_update(value),
            _ if id == Self::GAINS[2].id() => self.gains[2].apply_update(value),
            _ if id == Self::GAINS[3].id() => self.gains[3].apply_update(value),
            _ if id == Self::GAINS[4].id() => self.gains[4].apply_update(value),
            _ if id == Self::FREQUENCIES[0].id() => self.frequencies[0].apply_update(value),
            _ if id == Self::FREQUENCIES[1].id() => self.frequencies[1].apply_update(value),
            _ if id == Self::FREQUENCIES[2].id() => self.frequencies[2].apply_update(value),
            _ if id == Self::FREQUENCIES[3].id() => self.frequencies[3].apply_update(value),
            _ if id == Self::FREQUENCIES[4].id() => self.frequencies[4].apply_update(value),
            _ if id == Self::BANDWIDTHS[0].id() => self.bandwidths[0].apply_update(value),
            _ if id == Self::BANDWIDTHS[1].id() => self.bandwidths[1].apply_update(value),
            _ if id == Self::BANDWIDTHS[2].id() => self.bandwidths[2].apply_update(value),
            _ if id == Self::BANDWIDTHS[3].id() => self.bandwidths[3].apply_update(value),
            _ if id == Self::BANDWIDTHS[4].id() => self.bandwidths[4].apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };
        self.update_filter_coefficients()
    }
}
