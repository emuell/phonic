use super::AudioSource;
use crate::utils::resampler::{AudioResampler, ResamplingQuality, ResamplingSpec};

// -------------------------------------------------------------------------------------------------

/// Interpolation mode of the resampler.
pub type Quality = ResamplingQuality;

// -------------------------------------------------------------------------------------------------

/// A source which resamples the input source, either to adjust source's sample rate to a
/// target rate or to play back a source with a different pitch.
pub struct ResampledSource {
    source: Box<dyn AudioSource>,
    output_sample_rate: u32,
    resampler: AudioResampler,
    inp: ResampleBuffer,
    out: ResampleBuffer,
}

impl ResampledSource {
    /// Create a new resampled sources with the given sample rate adjustment.
    pub fn new<InputSource>(source: InputSource, output_sample_rate: u32, quality: Quality) -> Self
    where
        InputSource: AudioSource,
    {
        Self::new_with_speed(source, output_sample_rate, 1.0, quality)
    }
    /// Create a new resampled sources with the given sample rate and playback speed adjument.
    pub fn new_with_speed<InputSource>(
        source: InputSource,
        output_sample_rate: u32,
        speed: f64,
        quality: Quality,
    ) -> Self
    where
        InputSource: AudioSource,
    {
        const BUFFER_SIZE: usize = 1024;

        let spec = ResamplingSpec {
            channels: source.channel_count(),
            input_rate: source.sample_rate(),
            output_rate: (output_sample_rate as f64 / speed) as u32,
        };
        let inp_buf = vec![0.0; BUFFER_SIZE];
        let out_buf = vec![0.0; spec.output_size(BUFFER_SIZE)];
        Self {
            resampler: AudioResampler::new(quality, spec).unwrap(),
            source: Box::new(source),
            output_sample_rate,
            inp: ResampleBuffer {
                buf: inp_buf,
                start: 0,
                end: 0,
            },
            out: ResampleBuffer {
                buf: out_buf,
                start: 0,
                end: 0,
            },
        }
    }
}

impl AudioSource for ResampledSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        let mut total = 0;
        while total < output.len() {
            if self.out.is_empty() {
                // when there's no input, try fetch some from our source
                if self.inp.is_empty() {
                    let input_read = self.source.write(&mut self.inp.buf);
                    self.inp.start = 0;
                    self.inp.end = input_read;
                    self.inp.buf[input_read..].iter_mut().for_each(|s| *s = 0.0);
                }
                // run resampler to generate some output
                let input = &self.inp.buf[self.inp.start..self.inp.end];
                let output = &mut self.out.buf;
                let (inp_consumed, out_written) = self.resampler.process(input, output).unwrap();
                self.inp.start += inp_consumed;
                self.out.start = 0;
                self.out.end = out_written;
                if out_written == 0 {
                    // resampler produced no more output: we're done
                    break;
                }
            }
            // write resampler temp output to output
            let source = self.out.get();
            let target = &mut output[total..];
            let to_write = self.out.len().min(target.len());
            target[..to_write].copy_from_slice(&source[..to_write]);
            total += to_write;
            self.out.start += to_write;
        }
        total
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted() && self.inp.is_empty() && self.out.is_empty()
    }
}

// -------------------------------------------------------------------------------------------------

struct ResampleBuffer {
    buf: Vec<f32>,
    start: usize,
    end: usize,
}

impl ResampleBuffer {
    fn get(&self) -> &[f32] {
        &self.buf[self.start..self.end]
    }

    fn len(&self) -> usize {
        self.end - self.start
    }

    fn is_empty(&self) -> bool {
        self.start >= self.end
    }
}
