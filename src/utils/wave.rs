//! Helper functions to generate audio waveforms for display purposes.<br>
//!
//! ## Example
//!
//! Write mixed-down (mono) waveform from an audio file as SVG file using
//! [hound](https://github.com/ruuda/hound) as wave file reader.
//!
//! ```rust no_run
//! use svg::{node::element::{path::Data, Path}, Document};
//! use phonic::utils::waveform::mixed_down;
//!
//! # fn main() { || -> Result<(), Box<dyn std::error::Error>> {
//! #
//! // resolution/viewBox of the resulting SVG
//! const WIDTH: usize = 1024;
//! const HEIGHT: usize = 256;
//! const STROKE_WIDTH: usize = 1;
//!
//! // get specs and an interleaved, normalized buffer from some wavefile
//! let mut wave_reader = hound::WavReader::open("SOME_FILE.wav")?;
//! let specs = wave_reader.spec();
//! let buffer: Vec<f32> = wave_reader
//!     .samples::<i32>()
//!     .map(|v| v.unwrap() as f32 / (1 << (specs.bits_per_sample - 1)) as f32)
//!     .collect();
//!
//! // generate mixed-down, mono waveform data with WIDTH as resolution
//! let waveform_data = mixed_down(
//!     &buffer,
//!     specs.channels as usize,
//!     specs.sample_rate,
//!     WIDTH);
//!
//! // fit waveform points into our viewBox
//! let num_points = waveform_data.len();
//! let scale_x = move |v: f32| v * WIDTH as f32 / num_points as f32;
//! let scale_y = move |v: f32| (v + 1.0) * HEIGHT as f32 / 2.0;
//!
//! // create path from waveform points
//! let mut data = Data::new();
//! data = data.move_to((scale_x(0.0), scale_y(waveform_data[0].min)));
//! for (index, point) in waveform_data.into_iter().enumerate() {
//!     let x = scale_x(index as f32);
//!     data = data
//!         .line_to((x, scale_y(point.min)))
//!         .line_to((x, scale_y(point.max)));
//! }
//! let path = Path::new()
//!     .set("fill", "none")
//!     .set("stroke", "black")
//!     .set("stroke-width", STROKE_WIDTH)
//!     .set("d", data);
//!
//! // create svg document and add the path
//! let mut document = Document::new().set("viewBox", (0, 0, WIDTH, HEIGHT));
//! document = document.add(path);
//!
//! // write the document to a file
//! svg::save("SOME_WAVEFORM.svg", &document)?;
//! #
//! # Ok(()) }; }
//! ```

use std::time::Duration;

// -------------------------------------------------------------------------------------------------

/// A single point in a waveform view plot, which represents a condensed view of the audio data at
/// the specified time as min/max values.
/// The slice width is indirectly specified via the resolution parameter when generating the points.
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
/// given interleaved audio buffer and specs.
///
/// `Resolution` usually is the width in pixels that you want to draw the waveform into. The
/// returned points are guaranteed to be smaller or equal to the given resolution. When they are
/// smaller, there are less sample frames than the specified resolution present in the file.
/// The waveform must then be drawn upscaled. Else the resulting plot data will represent a
/// downscaled version of the original waveform data.
///
/// The resulting plot point's min/max values have the same range than the input signal.
pub fn mixed_down_waveform(
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
        for res_index in 0..resolution {
            let mut min = f32::MAX;
            let mut max = f32::MIN;
            let slice_start = (res_index as f32 * step_size) as usize * channel_count;
            let slice_end =
                (((res_index + 1) as f32 * step_size) as usize * channel_count).min(buffer.len());
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
                time: Duration::from_secs_f32(
                    slice_start as f32 / channel_count as f32 / samples_per_sec as f32,
                ),
                min,
                max,
            });
        }
    }
    waveform
}

// -------------------------------------------------------------------------------------------------

/// Generates display data for waveform plots from the given interleaved buffer with the given
/// signal specs and resolution.
///
/// See [`mixed_down_waveform`] for more info about the `resolution` parameter.
/// The resulting plot point's min/max values are used the same range as the buffer values.
pub fn multi_channel_waveform(
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
        for res_index in 0..resolution {
            let mut min = vec![f32::MAX; channel_count];
            let mut max = vec![f32::MIN; channel_count];
            let slice_start = (res_index as f32 * step_size) as usize * channel_count;
            let slice_end =
                (((res_index + 1) as f32 * step_size) as usize * channel_count).min(buffer.len());
            let slice = &buffer[slice_start..slice_end];
            for frame in slice.chunks_exact(channel_count) {
                for (channel_index, value) in frame.iter().enumerate() {
                    min[channel_index] = min[channel_index].min(*value);
                    max[channel_index] = max[channel_index].max(*value);
                }
            }
            let time = Duration::from_secs_f32(
                slice_start as f32 / channel_count as f32 / samples_per_sec as f32,
            );
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
    use super::mixed_down_waveform as mixed_down;
    use super::multi_channel_waveform as multi_channel;

    #[test]
    fn waveform() {
        let read_file = |file_path: &str| {
            let mut reader = hound::WavReader::open(file_path).unwrap();
            let buffer: Vec<f32> = reader.samples::<i32>().map(|v| v.unwrap() as f32).collect();
            let specs = reader.spec();
            (buffer, specs)
        };

        let (long_file_buffer, long_file_specs) = read_file("assets/YuaiLoop.wav");
        let (small_file_buffer, small_file_specs) = read_file("assets/AKWF_saw.wav");

        // downscale
        let mono_downscale_result = mixed_down(
            &long_file_buffer,
            long_file_specs.channels as usize,
            long_file_specs.sample_rate,
            1024,
        );
        assert_eq!(mono_downscale_result.len(), 1024);

        // upscale
        let mono_upscale_result = mixed_down(
            &small_file_buffer,
            small_file_specs.channels as usize,
            small_file_specs.sample_rate,
            1024,
        );
        assert!(mono_upscale_result.len() < 1024);

        // downscale
        let downscale_result = multi_channel(
            &long_file_buffer,
            long_file_specs.channels as usize,
            long_file_specs.sample_rate,
            1024,
        );
        assert_eq!(downscale_result[0].len(), 1024);

        // upscale
        let upscale_result = multi_channel(
            &small_file_buffer,
            small_file_specs.channels as usize,
            small_file_specs.sample_rate,
            1024,
        );
        assert!(upscale_result[0].len() < 1024);
    }
}
