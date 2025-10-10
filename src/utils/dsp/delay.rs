//! Delay buffers to delay or lookup signals.

// -------------------------------------------------------------------------------------------------

/// Multi channel delay line buffer with fractional delay time support.
#[derive(Debug, Default)]
pub struct DelayLine<const CHANNELS: usize> {
    buffer: Vec<f32>,
    buffer_mask: usize,
    write_pos: usize,
}

impl<const CHANNELS: usize> DelayLine<CHANNELS> {
    /// Create a new delay buffer with the given max delay time in sample frames.
    pub fn new(max_delay_frames: usize) -> Self {
        let (buffer, buffer_mask) = if max_delay_frames > 0 {
            let buffer_frames = max_delay_frames.next_power_of_two();
            (vec![0.0; buffer_frames * CHANNELS], buffer_frames - 1)
        } else {
            (Vec::new(), 0)
        };
        let write_pos = 0;
        Self {
            buffer,
            buffer_mask,
            write_pos,
        }
    }

    /// Reset the delay buffer and write position.
    pub fn flush(&mut self) {
        self.buffer.fill(0.0);
        self.write_pos = 0;
    }

    /// Process and add a single new sample frame and return the delayed sample with the given feedback.
    pub fn process_sample(
        &mut self,
        input: [f32; CHANNELS],
        feedback: f32,
        delay_pos: f32,
    ) -> [f32; CHANNELS] {
        let buffer_frames = self.buffer.len() / CHANNELS;
        debug_assert!(delay_pos >= 0.0 && (delay_pos.ceil() as usize) < buffer_frames);

        let read_pos = self.write_pos as f32 - delay_pos;

        let read_pos_floor = read_pos.floor();
        let fraction = read_pos - read_pos_floor;

        let index1 = read_pos_floor as isize;
        let index2 = index1 + 1;

        let mut output = [0.0; CHANNELS];
        #[allow(clippy::needless_range_loop)]
        for ch in 0..CHANNELS {
            let sample_index1 = ((index1 as usize) & self.buffer_mask) * CHANNELS + ch;
            let sample_index2 = ((index2 as usize) & self.buffer_mask) * CHANNELS + ch;

            let val1 = self.buffer[sample_index1];
            let val2 = self.buffer[sample_index2];

            output[ch] = val1 + (val2 - val1) * fraction;
        }

        let write_sample_index = self.write_pos * CHANNELS;
        for ch in 0..CHANNELS {
            self.buffer[write_sample_index + ch] = input[ch] + output[ch] * feedback;
        }
        self.write_pos = (self.write_pos + 1) & self.buffer_mask;

        output
    }
}

// -------------------------------------------------------------------------------------------------

/// Multi channel delay line which delays an input signal and keeps track of all channel's
/// peak values. Useful to lookup in e.g. compressors.
#[derive(Debug, Default)]
pub struct LookupDelayLine<const CHANNELS: usize> {
    buffer: Vec<f32>,
    write_pos: usize,
    buffer_mask: usize,
    delay_frames: usize,
    peak_value: f32,
    peak_pos: usize,
}

impl<const CHANNELS: usize> LookupDelayLine<CHANNELS> {
    pub fn new(sample_rate: u32, delay_time: f32) -> Self {
        let delay_frames = (delay_time * sample_rate as f32).ceil() as usize;

        let (buffer, buffer_mask) = if delay_frames > 0 {
            let buffer_frames = delay_frames.next_power_of_two();
            (vec![0.0; buffer_frames * CHANNELS], buffer_frames - 1)
        } else {
            (Vec::new(), 0)
        };

        let write_pos = 0;
        let peak_value = 0.0;
        let peak_pos = 0;
        Self {
            buffer,
            buffer_mask,
            write_pos,
            delay_frames,
            peak_value,
            peak_pos,
        }
    }

    /// Process one frame. Writes the input frame to the delay line and returns the delayed frame.
    pub fn process(&mut self, input_frame: &[f32; CHANNELS]) -> [f32; CHANNELS] {
        if self.delay_frames == 0 {
            return *input_frame;
        }

        // Read delayed frame from buffer
        let buffer_frames = self.buffer.len() / CHANNELS;
        let read_frame_index =
            (self.write_pos + buffer_frames - self.delay_frames) & self.buffer_mask;
        let read_sample_index = read_frame_index * CHANNELS;

        let mut delayed_frame = [0.0; CHANNELS];
        delayed_frame
            .copy_from_slice(&self.buffer[read_sample_index..read_sample_index + CHANNELS]);

        // Write current frame to buffer
        let write_sample_index = self.write_pos * CHANNELS;
        self.buffer[write_sample_index..write_sample_index + CHANNELS].copy_from_slice(input_frame);

        // update peak
        let peak_expired = self.peak_pos == read_frame_index;
        let new_peak = input_frame
            .iter()
            .fold(0.0f32, |max, &val| max.max(val.abs()));

        if new_peak >= self.peak_value {
            // New frame is the new peak.
            self.peak_value = new_peak;
            self.peak_pos = self.write_pos;
        } else if peak_expired {
            // Old peak expired and new frame is not the peak, so we must rescan.
            self.peak_value = 0.0;
            let buffer_frames = self.buffer.len() / CHANNELS;

            // The lookahead window is the last `delay_frames` that were written.
            // `write_pos` points to the most recently written frame.
            for i in 0..self.delay_frames {
                let frame_index = (self.write_pos + buffer_frames - i) & self.buffer_mask;
                let sample_index = frame_index * CHANNELS;
                let frame_peak = self.buffer[sample_index..sample_index + CHANNELS]
                    .iter()
                    .fold(0.0f32, |max, &val| max.max(val.abs()));
                if frame_peak >= self.peak_value {
                    self.peak_value = frame_peak;
                    self.peak_pos = frame_index;
                }
            }
        }

        // Increment write position
        self.write_pos = (self.write_pos + 1) & self.buffer_mask;

        delayed_frame
    }

    /// Returns the absolute peak value in the delay line from all channels.
    pub fn peak_value(&self) -> f32 {
        self.peak_value
    }
}
