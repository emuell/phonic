use std::time::Duration;

use crate::{
    file::FilePlaybackOptions, source::file::preloaded::PreloadedFileSource, AudioSource, Error,
};

// -------------------------------------------------------------------------------------------------

/// A single point in a waveform view plot, which represents a condensed view of the audio data at
/// the specified time as min/max values.
/// The slice width is indirectly specified via the resultion parameter when generating the points.
#[derive(Default, Clone)]
pub struct WaveformPoint {
    /// Start time this point refers to in the original sample buffer.
    pub time: Duration,
    /// The minimum of all values which are represented by this time slice.
    pub min: f32,
    /// The maximum of all values which are represented by this time slice.
    pub max: f32,
}

// -------------------------------------------------------------------------------------------------

/// Generates mixed-down mono display data for waveform plots with the given resolution from the
/// given audio file at the specified file path.
///
/// Resolution usually is the width in pixels that you want to draw the waveform into. The returned
/// points are guaranteed to be smaller or equal to the given resolution. When they are smaller,
/// there are less sample frames than the specified resolution present in the file. The waveform
/// must then be drawn upscaled. Else the resulting plot data will represent a downscaled version
/// of the original waveform data.
///
/// The resulting plot point's min/max values are normalized within a range of \[-1, 1.0\].
///
/// This function will return an error if the audio file failed to decode.
pub fn generate_mono_waveform_from_file(
    file_path: &str,
    resolution: usize,
) -> Result<Vec<WaveformPoint>, Error> {
    // load file as preloaded file source
    let source = PreloadedFileSource::new(file_path, None, FilePlaybackOptions::default())?;
    // generate waveform from the preloaded buffer and signal specs
    Ok(generate_mono_waveform_from_buffer(
        source.buffer(),
        source.channel_count(),
        source.sample_rate(),
        resolution,
    ))
}

// -------------------------------------------------------------------------------------------------

/// Generates mono (mixed down) display data for waveform plots with the given resolution from the
/// given sample buffer with the given signal specs and resolution.
///
/// See `generate_mono_waveform_from_file` for more info about the `resolution` parameter.
/// The resulting plot point's min/max values are used the same range as the buffer values.
pub fn generate_mono_waveform_from_buffer(
    buffer: &[f32],
    channel_count: usize,
    samples_per_sec: u32,
    resolution: usize,
) -> Vec<WaveformPoint> {
    let frame_count = buffer.len() / channel_count;
    let mut waveform = Vec::with_capacity(frame_count);

    // upscale
    if frame_count <= resolution {
        for (frame_index, frame) in buffer.chunks_exact(channel_count).enumerate() {
            let mono_value = frame
                .iter()
                .copied()
                .fold(0.0, |accum, iter| accum + iter / channel_count as f32);
            waveform.push(WaveformPoint {
                time: Duration::from_secs_f32(frame_index as f32 / samples_per_sec as f32),
                min: mono_value,
                max: mono_value,
            });
        }
    }
    // downscale
    else {
        let step_size = frame_count as f32 / resolution as f32;
        for index in 0..resolution {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let slice_start = (index as f32 * step_size) as usize;
            let slice_end = (((index + 1) as f32 * step_size) as usize).min(buffer.len());
            let slice = &buffer[slice_start..slice_end];
            for frame in slice.chunks_exact(channel_count) {
                let mono_value = frame
                    .iter()
                    .copied()
                    .fold(0.0, |accum, iter| accum + iter / channel_count as f32);
                min = min.min(mono_value);
                max = max.max(mono_value);
            }
            waveform.push(WaveformPoint {
                time: Duration::from_secs_f32(slice_start as f32 / samples_per_sec as f32),
                min,
                max,
            });
        }
    }
    waveform
}

// -------------------------------------------------------------------------------------------------

/// Generates display data for waveform plots with the given resolution, separately for each channel
/// in the audio file,  from the given audio file at the specified file path.
/// See `generate_mono_waveform` for more info about the `resolution` parameter and errors.
///
/// The resulting plot point's min/max values are normalized within a range of \[-1, 1.0\].
///
/// This function will return an error if the audio file failed to decode.
pub fn generate_waveform_from_file(
    file_path: &str,
    resolution: usize,
) -> Result<Vec<Vec<WaveformPoint>>, Error> {
    // load file as preloaded file source
    let source = PreloadedFileSource::new(file_path, None, FilePlaybackOptions::default())?;
    // generate waveform from preloaded buffer and signal specs
    Ok(generate_waveform_from_buffer(
        source.buffer(),
        source.channel_count(),
        source.sample_rate(),
        resolution,
    ))
}

// -------------------------------------------------------------------------------------------------

/// Generates display data for waveform plots from the given interleaved buffer with the given
/// signal specs and resolution.
///
/// See `generate_mono_waveform_from_file` for more info about the `resolution` parameter.
/// The resulting plot point's min/max values are used the same range as the buffer values.
pub fn generate_waveform_from_buffer(
    buffer: &[f32],
    channel_count: usize,
    samples_per_sec: u32,
    resolution: usize,
) -> Vec<Vec<WaveformPoint>> {
    let frame_count = buffer.len() / channel_count;
    let mut waveform = vec![Vec::with_capacity(frame_count); channel_count];

    // upscale
    if frame_count <= resolution {
        for (frame_index, frame) in buffer.chunks_exact(channel_count).enumerate() {
            let time = Duration::from_secs_f32(frame_index as f32 / samples_per_sec as f32);
            for (channel_index, value) in frame.iter().enumerate() {
                waveform[channel_index].push(WaveformPoint {
                    time,
                    min: *value,
                    max: *value,
                });
            }
        }
    }
    // downscale
    else {
        let step_size = frame_count as f32 / resolution as f32;
        for index in 0..resolution {
            let mut min = vec![f32::MAX; channel_count];
            let mut max = vec![f32::MIN; channel_count];
            let slice_start = (index as f32 * step_size) as usize;
            let slice_end = (((index + 1) as f32 * step_size) as usize).min(buffer.len());
            let slice = &buffer[slice_start..slice_end];
            for frame in slice.chunks_exact(channel_count) {
                for (channel_index, value) in frame.iter().enumerate() {
                    min[channel_index] = min[channel_index].min(*value);
                    max[channel_index] = max[channel_index].max(*value);
                }
            }
            let time = Duration::from_secs_f32(slice_start as f32 / samples_per_sec as f32);
            for channel_index in 0..channel_count {
                waveform[channel_index].push(WaveformPoint {
                    time,
                    min: min[channel_index],
                    max: max[channel_index],
                });
            }
        }
    }
    waveform
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform() {
        // downscale
        let mono_downscale_result =
            generate_mono_waveform_from_file("assets/BSQ_M14.wav", 1024).unwrap();
        assert_eq!(mono_downscale_result.len(), 1024);

        // upscale
        let mono_upscale_result =
            generate_mono_waveform_from_file("assets/AKWF_saw.wav", 1024).unwrap();
        assert!(mono_upscale_result.len() < 1024);

        // downscale
        let downscale_result = generate_waveform_from_file("assets/BSQ_M14.wav", 1024).unwrap();
        assert_eq!(downscale_result[0].len(), 1024);

        // upscale
        let upscale_result = generate_waveform_from_file("assets/AKWF_saw.wav", 1024).unwrap();
        assert!(upscale_result[0].len() < 1024);
    }
}
