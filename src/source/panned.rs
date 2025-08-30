use crate::utils::{
    apply_smoothed_panning, smoothed::ExponentialSmoothedValue, smoothed::SmoothedValue,
};

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which applies a pan factor to some other source's output
pub struct PannedSource {
    source: Box<dyn Source>,
    panning: ExponentialSmoothedValue,
}

impl PannedSource {
    pub fn new<InputSource>(source: InputSource, panning: f32) -> Self
    where
        InputSource: Source,
    {
        debug_assert!((-1.0..=1.0).contains(&panning), "Invalid panning factor");
        let sample_rate = source.sample_rate();
        let mut pan = ExponentialSmoothedValue::new(sample_rate);
        pan.init(panning);
        Self {
            source: Box::new(source),
            panning: pan,
        }
    }

    /// Update panning value
    #[allow(unused)]
    pub fn set_panning(&mut self, panning: f32) {
        self.panning.set_target(panning);
    }
}

impl Source for PannedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // write input source
        let written = self.source.write(output, time);
        // apply panning
        let channel_count = self.source.channel_count();
        apply_smoothed_panning(&mut output[..written], channel_count, &mut self.panning);
        written
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }
}
