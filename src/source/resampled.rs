use super::AudioSource;
use crate::utils::resampler::{AudioResampler, ResamplingQuality, ResamplingSpec};

// -------------------------------------------------------------------------------------------------

/// A source which resamples the input source to a target sample rate
pub struct ResampledSource {
    source: Box<dyn AudioSource>,
    resampler: AudioResampler,
    inp: ResampleBuffer,
    out: ResampleBuffer,
}

impl ResampledSource {
    pub fn new<InputSource>(
        source: InputSource,
        output_sample_rate: u32,
        quality: ResamplingQuality,
    ) -> Self
    where
        InputSource: AudioSource,
    {
        const BUFFER_SIZE: usize = 1024;

        let spec = ResamplingSpec {
            channels: source.channel_count(),
            input_rate: source.sample_rate(),
            output_rate: output_sample_rate,
        };
        let inp_buf = vec![0.0; BUFFER_SIZE];
        let out_buf = vec![0.0; spec.output_size(BUFFER_SIZE)];
        Self {
            resampler: AudioResampler::new(quality, spec).unwrap(),
            source: Box::new(source),
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
                if self.inp.is_empty() {
                    let n = self.source.write(&mut self.inp.buf);
                    self.inp.buf[n..].iter_mut().for_each(|s| *s = 0.0);
                    self.inp.start = 0;
                    self.inp.end = self.inp.buf.len();
                }
                let (inp_consumed, out_written) = self
                    .resampler
                    .process(&self.inp.buf[self.inp.start..], &mut self.out.buf)
                    .unwrap();
                self.inp.start += inp_consumed;
                self.out.start = 0;
                self.out.end = out_written;
            }
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
        self.resampler.spec.channels
    }

    fn sample_rate(&self) -> u32 {
        self.resampler.spec.output_rate
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted() && self.inp.is_empty()
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
