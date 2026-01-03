//! Delay buffers to delay or lookup signals.

use assume::assume;

// -------------------------------------------------------------------------------------------------

/// A fixed-length integer delay line.
///
/// This structure implements a basic circular buffer delay where the delay length is specified
/// in integer samples. While the delay length is passed to `process`, the implementation logic
/// (resetting the write pointer when it exceeds the delay) is designed for fixed delay lengths.
/// Changing the delay length during playback will likely cause discontinuities.
///
/// It uses a power-of-two backing buffer to allow efficient bitwise masking for memory safety,
/// but logically wraps the write pointer based on the requested `delay` size.
///
/// # Generics
/// * `CHANNELS`: The number of audio channels (e.g., 2 for stereo).
pub struct DelayLine<const CHANNELS: usize> {
    buffer: Vec<[f64; CHANNELS]>,
    buffer_mask: usize,
    write_pos: usize,
}

impl<const CHANNELS: usize> DelayLine<CHANNELS> {
    /// Create a new delay buffer with the given max delay time in sample frames.
    pub fn new(max_size: usize) -> Self {
        let buffer_frames = max_size.next_power_of_two();
        let buffer = vec![[0.0; CHANNELS]; buffer_frames];
        let buffer_mask = buffer_frames - 1;
        let write_pos = 0;
        Self {
            buffer,
            buffer_mask,
            write_pos,
        }
    }

    /// Reset the delay buffer and write position.
    pub fn flush(&mut self) {
        self.buffer.fill([0.0; CHANNELS]);
        self.write_pos = 0;
    }

    /// Process with a variable delay limit, ignoring the internal fixed size.
    /// This implements a "write then read" logic with explicit wrapping at `delay`.
    pub fn process(&mut self, delay: usize, input: [f64; CHANNELS]) -> [f64; CHANNELS] {
        debug_assert!(
            delay < self.buffer.len() - 1,
            "Delay must be < {} but is {}",
            self.buffer.len() - 1,
            delay
        );

        // Hint to optimizer: mask is always valid for this buffer.
        assume!(unsafe: self.buffer_mask < self.buffer.len());
        self.write_pos &= self.buffer_mask;
        self.buffer[self.write_pos] = input;

        self.write_pos = (self.write_pos + 1) & self.buffer_mask;
        if self.write_pos > delay {
            self.write_pos = 0;
        }

        self.buffer[self.write_pos]
    }
}

// -------------------------------------------------------------------------------------------------

/// A multi-channel delay line with fractional reads and optional feedback.
///
/// This structure allows for reading from the delay buffer at non-integer positions using
/// linear interpolation. This enables smooth modulation of the delay time.
///
/// # Generics
/// * `CHANNELS`: The number of audio channels.
#[derive(Debug, Default)]
pub struct InterpolatedDelayLine<const CHANNELS: usize> {
    buffer: Vec<[f64; CHANNELS]>,
    buffer_mask: usize,
    write_pos: usize,
}

impl<const CHANNELS: usize> InterpolatedDelayLine<CHANNELS> {
    /// Create a new delay buffer with the given max delay time in sample frames.
    pub fn new(max_size: usize) -> Self {
        let buffer_frames = max_size.next_power_of_two();
        let buffer = vec![[0.0; CHANNELS]; buffer_frames];
        let buffer_mask = buffer_frames - 1;
        let write_pos = 0;
        Self {
            buffer,
            buffer_mask,
            write_pos,
        }
    }

    /// Reset the delay buffer and write position.
    pub fn flush(&mut self) {
        self.buffer.fill([0.0; CHANNELS]);
        self.write_pos = 0;
    }

    /// Process and add a single new sample frame and return the delayed sample with the given feedback.
    #[inline]
    pub fn process(
        &mut self,
        input: [f32; CHANNELS],
        feedback: f32,
        delay: f32,
    ) -> [f32; CHANNELS] {
        debug_assert!(
            delay >= 0.0 && (delay.ceil() as usize) < self.buffer.len() - 1,
            "Delay must be > 0 and < {} but is {}",
            self.buffer.len() - 1,
            delay
        );

        let read_pos = self.write_pos as f64 - delay as f64;

        let read_pos_floor = read_pos.floor();
        let fraction = read_pos - read_pos_floor;

        let index1 = read_pos_floor as isize;
        let index2 = index1 + 1;

        let mut output = [0.0; CHANNELS];

        // Hint to optimizer: mask is always valid for this buffer.
        assume!(unsafe: self.buffer_mask < self.buffer.len());
        let read_idx1 = (index1 as usize) & self.buffer_mask;
        let read_idx2 = (index2 as usize) & self.buffer_mask;

        let frame1 = self.buffer[read_idx1];
        let frame2 = self.buffer[read_idx2];

        for ch in 0..CHANNELS {
            let val1 = frame1[ch];
            let val2 = frame2[ch];

            output[ch] = (val1 + (val2 - val1) * fraction) as f32;
        }

        let write_sample_index = self.write_pos & self.buffer_mask;
        let mut write_frame = [0.0; CHANNELS];
        for ch in 0..CHANNELS {
            write_frame[ch] = input[ch] as f64 + output[ch] as f64 * feedback as f64;
        }
        self.buffer[write_sample_index] = write_frame;

        self.write_pos = (self.write_pos + 1) & self.buffer_mask;

        output
    }
}

// -------------------------------------------------------------------------------------------------

/// A lookahead delay line that tracks peak values.
///
/// This delay line delays the input signal by a fixed amount while simultaneously
/// maintaining the maximum peak amplitude currently present in the buffer.
///
/// Can be used for dynamics processors: Lookahead Limiters, Compressors, and Gates.
/// It allows the processor to "see" upcoming peaks and reduce gain *before* the peak occurs,
/// preventing clipping or allowing for smoother attack characteristics.
///
/// # Generics
/// * `CHANNELS`: The number of audio channels.
#[derive(Debug, Default)]
pub struct LookupDelayLine<const CHANNELS: usize> {
    buffer: Vec<[f64; CHANNELS]>,
    write_pos: usize,
    buffer_mask: usize,
    delay_frames: usize,
    peak_value: f64,
    peak_pos: usize,
}

impl<const CHANNELS: usize> LookupDelayLine<CHANNELS> {
    pub fn new(sample_rate: u32, delay_time: f32) -> Self {
        let delay_frames = (delay_time * sample_rate as f32).ceil() as usize;

        let (buffer, buffer_mask) = if delay_frames > 0 {
            let buffer_frames = delay_frames.next_power_of_two();
            (vec![[0.0; CHANNELS]; buffer_frames], buffer_frames - 1)
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

        // Hint to optimizer: mask is always valid for this buffer.
        assume!(unsafe: self.buffer_mask < self.buffer.len());

        // Read delayed frame from buffer
        let buffer_frames = self.buffer.len();
        let read_frame_index =
            (self.write_pos + buffer_frames - self.delay_frames) & self.buffer_mask;

        let delayed_frame_f64 = self.buffer[read_frame_index];
        let mut delayed_frame = [0.0; CHANNELS];
        for ch in 0..CHANNELS {
            delayed_frame[ch] = delayed_frame_f64[ch] as f32;
        }

        // Write current frame to buffer
        let write_frame_index = self.write_pos & self.buffer_mask;
        let mut write_frame = [0.0; CHANNELS];
        for ch in 0..CHANNELS {
            write_frame[ch] = input_frame[ch] as f64;
        }
        self.buffer[write_frame_index] = write_frame;

        // update peak
        let peak_expired = self.peak_pos == read_frame_index;
        let new_peak = input_frame
            .iter()
            .fold(0.0f64, |max, &val| max.max(val.abs() as f64));

        if new_peak >= self.peak_value {
            // New frame is the new peak.
            self.peak_value = new_peak;
            self.peak_pos = self.write_pos;
        } else if peak_expired {
            // Old peak expired and new frame is not the peak, so we must rescan.
            self.peak_value = 0.0;
            let buffer_frames = self.buffer.len();

            // The lookahead window is the last `delay_frames` that were written.
            // `write_pos` points to the most recently written frame.
            for i in 0..self.delay_frames {
                let frame_index = (self.write_pos + buffer_frames - i) & self.buffer_mask;
                let frame = self.buffer[frame_index];
                let frame_peak = frame.iter().fold(0.0f64, |max, &val| max.max(val.abs()));
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
        self.peak_value as f32
    }
}

// -------------------------------------------------------------------------------------------------

/// Generalized multi-channel allpass filter delay line.
///
/// An allpass filter passes all frequencies with unity gain (magnitude response is 1.0)
/// but alters the phase relationship of frequencies. This implementation is based on the
/// [Schroeder Allpass](https://ccrma.stanford.edu/~jos/pasp/Schroeder_Reverberators.html) structure.
///
/// # Generics
/// * `CHANNELS`: Number of audio channels (e.g., 2 for Stereo).
pub struct AllpassDelayLine<const CHANNELS: usize> {
    buffer: Vec<[f64; CHANNELS]>,
    delay: usize,
    write_pos: usize,
}

impl<const CHANNELS: usize> AllpassDelayLine<CHANNELS> {
    pub fn new(max_size: usize) -> Self {
        Self {
            buffer: vec![[0.0; CHANNELS]; max_size],
            delay: 0,
            write_pos: 0,
        }
    }

    pub fn flush(&mut self) {
        self.buffer.fill([0.0; CHANNELS]);
        self.write_pos = 0;
    }

    pub fn set_delay(&mut self, delay: usize) {
        debug_assert!(
            delay < self.buffer.len() - 1,
            "Delay must be < {} but is {}",
            self.buffer.len() - 1,
            delay
        );
        self.delay = delay.min(self.buffer.len() - 1);
    }

    #[inline]
    pub fn process(&mut self, input: [f64; CHANNELS]) -> [f64; CHANNELS] {
        let mut read_pos = self.write_pos + 1;
        if read_pos > self.delay {
            read_pos = 0;
        }

        // Hint to optimizer: `self.delay` is clamped to `self.buffer.len() - 1` in the
        // setter, so `read_pos` and `self.write_pos` are always valid here.
        assume!(unsafe: read_pos < self.buffer.len());
        assume!(unsafe: self.write_pos < self.buffer.len());

        let delayed = self.buffer[read_pos];

        let mut output = [0.0; CHANNELS];
        let mut write_frame = [0.0; CHANNELS];

        for ch in 0..CHANNELS {
            let val_in = input[ch];
            let buf = val_in - (delayed[ch] * 0.5);
            write_frame[ch] = buf;
            output[ch] = buf * 0.5;
        }

        self.buffer[self.write_pos] = write_frame;

        self.write_pos += 1;
        if self.write_pos > self.delay {
            self.write_pos = 0;
        }

        let new_delayed = self.buffer[self.write_pos];
        for ch in 0..CHANNELS {
            output[ch] += new_delayed[ch];
        }

        output
    }
}
