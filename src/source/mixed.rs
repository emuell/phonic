use super::AudioSource;
use crate::utils::resampler::ResamplingQuality;
use crossbeam_queue::SegQueue;

// -------------------------------------------------------------------------------------------------

struct PlayingSource {
    is_active: bool,
    source: Box<dyn AudioSource>,
}

// -------------------------------------------------------------------------------------------------

#[derive(Default)]
pub struct MixedSource {
    pending_sources: SegQueue<Box<dyn AudioSource>>,
    sources: Vec<PlayingSource>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        const BUFFER_SIZE: usize = 8 * 1024;
        Self {
            pending_sources: SegQueue::new(),
            sources: Vec::new(),
            channel_count,
            sample_rate,
            temp_out: vec![0.0; BUFFER_SIZE],
        }
    }

    pub fn add<T>(&self, source: T)
    where
        T: AudioSource,
    {
        // add to pending_sources which will be consumed in the next write
        if source.sample_rate() != self.sample_rate() {
            // convert source sample-rate to ours
            let source = source.resampled(self.sample_rate(), ResamplingQuality::SincMediumQuality);
            if source.channel_count() != self.channel_count() {
                let source = source.channel_mapped(self.channel_count());
                self.pending_sources.push(Box::new(source));
            } else {
                self.pending_sources.push(Box::new(source));
            }
        } else if source.channel_count() != self.channel_count() {
            // convert source channel mapping to ours
            let source = source.channel_mapped(self.channel_count());
            self.pending_sources.push(Box::new(source));
        } else {
            self.pending_sources.push(Box::new(source));
        }
    }
}

impl AudioSource for MixedSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        let channel_count = self.channel_count();
        // clear output as we're only adding below
        for o in output.iter_mut() {
            *o = 0_f32;
        }
        // add pending sources
        while let Some(source) = self.pending_sources.pop() {
            self.sources.push(PlayingSource {
                is_active: true,
                source,
            });
        }
        // write all sources
        let mut max_written = 0usize;
        for playing_source in self.sources.iter_mut() {
            let source = &mut playing_source.source;
            debug_assert_eq!(
                channel_count,
                source.channel_count(),
                "expecting same channel layout"
            );
            let mut total_written = 0;
            'source: while total_written < output.len() {
                // run source on temp_out until we've filled up the whole final output
                let remaining = output.len() - total_written;
                let to_write = remaining.min(self.temp_out.len());
                let written = source.write(&mut self.temp_out[..to_write]);
                if written == 0 {
                    // no more samples present in this source
                    playing_source.is_active = false;
                    break 'source;
                }
                // add output of the source to the final output
                let remaining_out = &mut output[total_written..];
                let written_out = &self.temp_out[..written];
                for (o, i) in remaining_out.iter_mut().zip(written_out) {
                    *o += *i;
                }
                total_written += written;
            }
            max_written = max_written.max(total_written);
        }
        // remove stopped sources
        self.sources.retain(|s| s.is_active);
        // return dirty output len
        max_written
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
