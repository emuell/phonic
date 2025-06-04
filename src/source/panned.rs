use crate::utils::panning_factors;

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// A source which applies a pan factor to some other source's output
pub struct PannedSource {
    source: Box<dyn Source>,
    panning: f32,
}

impl PannedSource {
    pub fn new<InputSource>(source: InputSource, panning: f32) -> Self
    where
        InputSource: Source,
    {
        debug_assert!((-1.0..=1.0).contains(&panning), "Invalid panning factor");
        Self {
            source: Box::new(source),
            panning,
        }
    }
}

impl Source for PannedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // write input source
        let written = self.source.write(output, time);
        // apply panning
        let channel_count = self.source.channel_count();
        if self.panning.abs() > 0.001 {
            let written_out = &mut output[0..written];
            let (pan_l, pan_r) = panning_factors(self.panning);
            match channel_count {
                1 => {
                    // can't apply panning on mono sources
                }
                2 => {
                    for o in written_out.chunks_exact_mut(2) {
                        o[0] *= pan_l; // left
                        o[1] *= pan_r; // right
                    }
                }
                _ => {
                    for o in written_out.chunks_exact_mut(channel_count) {
                        // TODO: handle multi channel layouts
                        o[0] *= pan_l; // left
                        o[1] *= pan_r; // right
                    }
                }
            }
        }
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
