use std::time::Duration;

use crate::{
    file::FilePlaybackOptions, source::file::preloaded::PreloadedFileSource, AudioSource, Error,
};

// -------------------------------------------------------------------------------------------------

/// A single point in a waveform plot.
#[derive(Default)]
pub struct WaveformPoint {
    /// starting time of min/max within the sample buffer.
    pub time: Duration,
    /// the minimum of all values which are represented by this time.
    pub min: f32,
    /// the maximum of all values which are represented by this time.
    pub max: f32,
}

// -------------------------------------------------------------------------------------------------

/// Generates mono display data for waveform plots with the given resolution.
///
/// Resolution usually is the width in pixels that you want to draw the waveform into. Oversampling
/// may help to smoothen the display a bit. The returned points are guaranteed to be smaller or
/// equal the given resolution. They will be smaller when there are less frames present in the sample
/// than the specified resolution.
///
/// This function will return an error if the sample failed to decode.
pub fn generate_waveform(file_path: &str, resolution: usize) -> Result<Vec<WaveformPoint>, Error> {
    // load file as preloaded file source
    let source = PreloadedFileSource::new(file_path, None, FilePlaybackOptions::default())?;

    // get buffer and signal specs from source
    let buffer = source.buffer();
    let samples_per_channel = source.channel_count();
    let frame_count = buffer.len() / samples_per_channel;
    let frames_per_second = source.sample_rate() as f32;
    let samples_per_resolution_step = frame_count / resolution;

    // upscale
    if samples_per_resolution_step <= 1 {
        let mut waveform = Vec::with_capacity(buffer.len() / samples_per_channel);
        for (frame_index, frame) in buffer.chunks_exact(samples_per_channel).enumerate() {
            let mono_value = frame
                .iter()
                .copied()
                .fold(0.0, |accum, iter| accum + iter / samples_per_channel as f32);
            waveform.push(WaveformPoint {
                time: Duration::from_secs_f32(frame_index as f32 / frames_per_second),
                min: mono_value,
                max: mono_value,
            });
        }
        Ok(waveform)
    }
    // downscale
    else {
        let mut index = 0;
        let mut waveform = Vec::with_capacity(resolution);
        while index < resolution {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let slice_start = index * samples_per_resolution_step;
            let slice_end = ((index + 1) * samples_per_resolution_step).min(buffer.len());
            let slice = &buffer[slice_start..slice_end];
            for frame in slice.chunks_exact(samples_per_channel) {
                let mono_value = frame
                    .iter()
                    .copied()
                    .fold(0.0, |accum, iter| accum + iter / samples_per_channel as f32);
                min = min.min(mono_value);
                max = max.max(mono_value);
            }
            waveform.push(WaveformPoint {
                time: Duration::from_secs_f32(slice_start as f32 / frames_per_second),
                min,
                max,
            });
            index += 1;
        }
        Ok(waveform)
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform() {
        // downscale
        let downscale_result = generate_waveform("assets/BSQ_M14.wav", 1024).unwrap();
        assert_eq!(downscale_result.len(), 1024);

        // upscale
        let upscale_result = generate_waveform("assets/AKWF_saw.wav", 1024).unwrap();
        assert!(upscale_result.len() < 1024);
    }
}
