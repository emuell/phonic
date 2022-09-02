use std::sync::Mutex;

use super::{mapped::ChannelMappedSource, resampled::ResampledSource, AudioSource};
use crate::utils::resampler::ResamplingQuality;

// -------------------------------------------------------------------------------------------------

pub struct MixedSource {
    sources: Mutex<Vec<Box<dyn AudioSource>>>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    pub fn new() -> Self {
        const BUFFER_SIZE: usize = 1024; // TODO: get audio sink's buffer size here
        Self {
            sources: Mutex::new(Vec::new()),
            channel_count: 2,
            sample_rate: 48000,
            temp_out: vec![0.0; BUFFER_SIZE],
        }
    }

    pub fn add(&self, source: Box<dyn AudioSource>) {
        let mut converted_source: Box<dyn AudioSource> = source;
        if converted_source.sample_rate() != self.sample_rate() {
            // convert source sample-rate to ours
            converted_source = Box::new(ResampledSource::new(
                converted_source,
                self.sample_rate(),
                ResamplingQuality::SincMediumQuality,
            ));
        }
        if converted_source.channel_count() != self.channel_count() {
            // convert source channel mapping to ours
            converted_source = Box::new(ChannelMappedSource::new(
                converted_source,
                self.channel_count(),
            ));
        }
        self.sources.lock().unwrap().push(converted_source);
    }
}

impl AudioSource for MixedSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        output.iter_mut().for_each(|v| *v = 0f32);

        let mut max_written = 0usize;
        for source in self.sources.lock().unwrap().iter_mut() {
            let to_write = output.len();
            let mut total_written = 0;
            while total_written < to_write {
                let remaining = to_write - total_written;
                let temp_out_size = remaining.min(self.temp_out.len() - 1);
                let written = source.write(&mut self.temp_out[..temp_out_size]);
                let remaining_out = &mut output[total_written..];
                for (i, v) in self.temp_out[..written].iter_mut().enumerate() {
                    remaining_out[i] += *v;
                }
                total_written += written;
            }
            max_written = max_written.max(total_written);
        }
        max_written
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}
