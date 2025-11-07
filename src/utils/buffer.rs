//! Provides utilities for converting between planar and interleaved audio buffers,
//! SIMD-accelerated buffer operations, and safe abstractions for working with interleaved audio data.

use pulp::Simd;

// -------------------------------------------------------------------------------------------------

/// Copy the given planar buffer into an interleaved one.
/// The planar buffer's layout defines layout of the interleaved buffer (channel and frame count).
/// The interleaved buffer must be large enough to fit the planar buffer.
pub fn planar_to_interleaved(planar: &[Vec<f32>], mut interleaved: &mut [f32]) {
    let channel_count = planar.len();
    let frame_count = planar[0].len();
    debug_assert!(
        interleaved.len() >= frame_count * channel_count,
        "Buffer size mismatch"
    );
    match channel_count {
        1 => {
            copy_buffers(&mut interleaved[..frame_count], &planar[0]);
        }
        2 => {
            let left = &planar[0];
            let right = &planar[1];
            let frames = interleaved.as_frames_mut::<2>();
            for (frame, (l, r)) in frames.iter_mut().zip(left.iter().zip(right.iter())) {
                frame[0] = *l;
                frame[1] = *r;
            }
        }
        _ => {
            for (channel_index, channel) in planar.iter().enumerate() {
                for (p, i) in channel
                    .iter()
                    .zip(interleaved.chunks_exact_mut(channel_count))
                {
                    i[channel_index] = *p;
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Copy the given interleaved buffer into a planar one.
/// The planar buffer's layout defines layout of the interleaved buffer (channel and frame count).
/// The interleaved buffer must be large enough to fill the planar buffer.
pub fn interleaved_to_planar(interleaved: &[f32], planar: &mut [Vec<f32>]) {
    let channel_count = planar.len();
    let frame_count = planar[0].len();
    debug_assert!(
        interleaved.len() >= frame_count * channel_count,
        "Buffer size mismatch"
    );
    match channel_count {
        1 => {
            copy_buffers(&mut planar[0], &interleaved[..frame_count]);
        }
        2 => {
            let frames = interleaved.as_frames::<2>();
            let left = &mut planar[0];
            for (l, frame) in left.iter_mut().zip(frames) {
                *l = frame[0];
            }
            let right = &mut planar[1];
            for (r, frame) in right.iter_mut().zip(frames) {
                *r = frame[1];
            }
        }
        _ => {
            for (channel_index, channel) in planar.iter_mut().enumerate() {
                for (p, i) in channel
                    .iter_mut()
                    .zip(interleaved.chunks_exact(channel_count))
                {
                    *p = i[channel_index];
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[pulp::with_simd(clear_buffer = pulp::Arch::new())]
#[inline(always)]
/// Dest = 0.0
pub fn clear_buffer_with_simd<S: Simd>(simd: S, dest: &mut [f32]) {
    let (head, tail) = S::as_mut_simd_f32s(dest);
    let zero = simd.splat_f32s(0.0);
    for x in head.iter_mut() {
        *x = zero;
    }
    for x in tail.iter_mut() {
        *x = 0.0;
    }
}

#[pulp::with_simd(scale_buffer = pulp::Arch::new())]
#[inline(always)]
/// Dest *= Value
pub fn scale_buffer_with_simd<S: Simd>(simd: S, dest: &mut [f32], value: f32) {
    let (head, tail) = S::as_mut_simd_f32s(dest);
    let valuef32 = simd.splat_f32s(value);
    for x in head.iter_mut() {
        *x = simd.mul_f32s(*x, valuef32);
    }
    for x in tail.iter_mut() {
        *x *= value;
    }
}

#[pulp::with_simd(add_buffers = pulp::Arch::new())]
#[inline(always)]
/// Dest += Source
pub fn add_buffers_with_simd<'a, S: Simd>(simd: S, dest: &'a mut [f32], source: &'a [f32]) {
    assert!(
        dest.len() == source.len(),
        "added buffers should have the same size"
    );
    let (head1, tail1) = S::as_mut_simd_f32s(dest);
    let (head2, tail2) = S::as_simd_f32s(source);
    for (x1, x2) in head1.iter_mut().zip(head2) {
        *x1 = simd.add_f32s(*x1, *x2);
    }
    for (x1, x2) in tail1.iter_mut().zip(tail2) {
        *x1 += *x2;
    }
}

#[pulp::with_simd(copy_buffers = pulp::Arch::new())]
#[inline(always)]
/// Dest = Source
pub fn copy_buffers_with_simd<'a, S: Simd>(_simd: S, dest: &'a mut [f32], source: &'a [f32]) {
    assert!(
        dest.len() == source.len(),
        "copied buffers should have the same size"
    );
    let (head1, tail1) = S::as_mut_simd_f32s(dest);
    let (head2, tail2) = S::as_simd_f32s(source);
    for (x1, x2) in head1.iter_mut().zip(head2) {
        *x1 = *x2;
    }
    for (x1, x2) in tail1.iter_mut().zip(tail2) {
        *x1 = *x2;
    }
}

#[pulp::with_simd(max_abs_sample = pulp::Arch::new())]
#[inline(always)]
/// Find the maximum absolute value in a buffer using SIMD
pub fn max_abs_sample_with_simd<S: Simd>(simd: S, buffer: &[f32]) -> f32 {
    let (head, tail) = S::as_simd_f32s(buffer);

    // Process SIMD lanes
    let mut max_vec = simd.splat_f32s(0.0);
    for &x in head {
        let abs_x = simd.abs_f32s(x);
        max_vec = simd.max_f32s(max_vec, abs_x);
    }

    // Reduce SIMD vector to scalar
    let lanes = simd.reduce_max_f32s(max_vec);

    // Process remaining scalar elements
    let mut max_scalar = lanes;
    for &x in tail {
        max_scalar = max_scalar.max(x.abs());
    }

    max_scalar
}

// -------------------------------------------------------------------------------------------------

/// Provides safe and efficient methods to access interleaved audio data, such as iterating over
/// frames or individual channels, without needing to perform manual index calculations.
/// It is implemented for common buffer types like `&[f32]` and `Vec<f32>`.
///
/// # Examples
///
/// ```
/// use phonic::utils::buffer::InterleavedBuffer;
///
/// let buffer: &[f32] = &[
///     0.1, 0.2, // Frame 0
///     0.3, 0.4, // Frame 1
/// ];
///
/// // View as frames
/// let frames = buffer.as_frames::<2>();
/// assert_eq!(frames, &[[0.1, 0.2], [0.3, 0.4]]);
///
/// // Iterate over channels
/// let mut channels = buffer.channels(2);
/// let left: Vec<_> = channels.next().unwrap().copied().collect();
/// let right: Vec<_> = channels.next().unwrap().copied().collect();
/// assert_eq!(left, &[0.1, 0.3]);
/// assert_eq!(right, &[0.2, 0.4]);
/// ```
pub trait InterleavedBuffer<'a> {
    /// Raw access to the buffer slice.
    fn buffer(&self) -> &'a [f32];

    /// Iterate by channels. This returns one iterator per channel, each yielding the samples
    /// of that channel across all frames.
    ///
    /// Note: Prefer using `frames` or `as_frames` for hot paths as it's more efficient.
    fn channels(
        &self,
        channel_count: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = &'a f32> + 'a> + 'a {
        let buffer = self.buffer();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(channel_count),
            "channels: buffer length ({buffer_len}) must be divisible by channel count ({channel_count})",
        );
        (0..channel_count).map(move |i| buffer.iter().skip(i).step_by(channel_count))
    }

    /// Iterate over frames. Each frame yields `channel_count` samples.
    ///
    /// Note: Prefer using `as_frames` for hot paths. They avoid iterator overhead and yield
    /// fixed-size arrays per frame for better compiler optimization and cache locality.
    fn frames(
        &self,
        channel_count: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = &'a f32> + 'a> + 'a {
        let buffer = self.buffer();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(channel_count),
            "frames: buffer length ({buffer_len}) must be divisible by channel count ({channel_count})",
        );
        self.buffer()
            .chunks_exact(channel_count)
            .map(move |channel_chunk| channel_chunk.iter().take(channel_count))
    }

    /// View the interleaved samples as contiguous frames of size `CHANNEL_COUNT`.
    ///
    /// Constraints:
    /// - The buffer length must be divisible by `CHANNEL_COUNT` (i.e. no remainder).
    ///
    /// Typical usage is `CHANNEL_COUNT == self.channel_count()`, in which case each array is one frame.
    fn as_frames<const CHANNEL_COUNT: usize>(&self) -> &'a [[f32; CHANNEL_COUNT]] {
        let buffer = self.buffer();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(CHANNEL_COUNT),
            "as_frames: buffer length ({buffer_len}) must be divisible by N ({CHANNEL_COUNT})",
        );
        let frames_count = self.buffer().len() / CHANNEL_COUNT;
        let ptr = buffer.as_ptr() as *const [f32; CHANNEL_COUNT];
        unsafe { std::slice::from_raw_parts(ptr, frames_count) }
    }
}

// -------------------------------------------------------------------------------------------------

/// Extends [`InterleavedBuffer`] with methods for modifying the buffer's contents.
/// It provides safe and efficient ways to get mutable access to frames or individual channels.
///
/// Like [`InterleavedBuffer`], it is implemented for common buffer types that allow mutation,
/// such as `&mut [f32]` and `Vec<f32>`.
///
/// # Examples
///
/// ```
/// use phonic::utils::buffer::{InterleavedBuffer, InterleavedBufferMut};
///
/// let mut buffer: Vec<f32> = vec![
///     0.1, 0.2, // Frame 0
///     0.3, 0.4, // Frame 1
/// ];
///
/// // Get mutable access to sample frames and modify them
/// for frame in buffer.as_frames_mut::<2>() {
///     frame[0] *= 2.0; // Double the left channel
///     frame[1] *= 0.5; // Halve the right channel
/// }
/// assert_eq!(buffer, vec![
///     0.2, 0.1, // Frame 0
///     0.6, 0.2  // Frame 1
/// ]);
/// ```
pub trait InterleavedBufferMut<'a>: InterleavedBuffer<'a> {
    /// Raw mut access to the buffer slice.
    fn buffer_mut(&mut self) -> &'a mut [f32];

    /// Mutable channel-wise iteration.
    ///
    /// Returns an iterator over channels, each yielding mutable samples across all frames.
    /// This uses internal pointer arithmetic to safely create non-overlapping mutable references
    /// into the interleaved buffer.
    ///
    /// Note: Prefer using `frames_mut` or `as_frames_mut` for hot paths as it's more efficient.
    fn channels_mut(
        &mut self,
        channel_count: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = &'a mut f32> + 'a> + 'a {
        let buffer = self.buffer_mut();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(channel_count),
            "channels_mut: buffer length ({buffer_len}) must be divisible by channel count ({channel_count})",
        );
        let frame_count = buffer_len / channel_count;
        let ptr = buffer.as_mut_ptr();
        (0..channel_count).map(move |ch_idx| {
            (0..frame_count).map(move |fr_idx| {
                // SAFETY: The outer iterator produces one iterator for each channel.
                // Each inner iterator accesses samples of one channel, which do not overlap with
                // samples of other channels. The borrow checker cannot prove this, so we use
                // unsafe. The lifetime 'a ensures that the returned iterators do not outlive
                // the buffer.
                unsafe { &mut *ptr.add(ch_idx + fr_idx * channel_count) }
            })
        })
    }

    /// Iterate over frames mutably. Each frame yields `channel_count` mutable samples.
    ///
    /// Note: Prefer using `as_frames_mut` for hot paths. They avoid iterator overhead and yield
    /// fixed-size arrays per frame for better compiler optimization and cache locality.
    fn frames_mut(
        &mut self,
        channel_count: usize,
    ) -> impl Iterator<Item = impl Iterator<Item = &'a mut f32> + 'a> + 'a {
        let buffer = self.buffer_mut();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(channel_count),
            "frames_mut: buffer length ({buffer_len}) must be divisible by channel count ({channel_count})",
        );
        buffer
            .chunks_exact_mut(channel_count)
            .map(move |channel_chunk| channel_chunk.iter_mut().take(channel_count))
    }

    /// Mutable view of the interleaved samples as contiguous frames of size `CHANNEL_COUNT`.
    ///
    /// Constraints:
    /// - The buffer length must be divisible by `CHANNEL_COUNT` (i.e. no remainder).
    ///
    /// Typical usage is `CHANNEL_COUNT == self.channel_count()`, in which case each array is one frame.
    fn as_frames_mut<const CHANNEL_COUNT: usize>(&mut self) -> &'a mut [[f32; CHANNEL_COUNT]] {
        let buffer = self.buffer_mut();
        let buffer_len = buffer.len();
        assert!(
            buffer_len.is_multiple_of(CHANNEL_COUNT),
            "as_frames_mut: buffer length ({buffer_len}) must be divisible by N ({CHANNEL_COUNT})",
        );
        let frames_count = self.buffer().len() / CHANNEL_COUNT;
        let ptr = buffer.as_mut_ptr() as *mut [f32; CHANNEL_COUNT];
        unsafe { std::slice::from_raw_parts_mut(ptr, frames_count) }
    }
}

// -------------------------------------------------------------------------------------------------

impl<'a> InterleavedBuffer<'a> for &'a [f32] {
    fn buffer(&self) -> &'a [f32] {
        self
    }
}

impl<'a> InterleavedBuffer<'a> for &'a mut [f32] {
    fn buffer(&self) -> &'a [f32] {
        // SAFETY: The lifetime 'a is tied to the underlying slice, which is valid.
        // The compiler incorrectly restricts the lifetime to that of `&self`.
        unsafe { &*(*self as *const [f32]) }
    }
}

impl<'a> InterleavedBufferMut<'a> for &'a mut [f32] {
    fn buffer_mut(&mut self) -> &'a mut [f32] {
        // SAFETY: The lifetime 'a is tied to the underlying slice, which is valid.
        // The compiler incorrectly restricts the lifetime to that of `&mut self`.
        unsafe { &mut *(*self as *mut [f32]) }
    }
}

impl<'a> InterleavedBuffer<'a> for Vec<f32> {
    fn buffer(&self) -> &'a [f32] {
        unsafe { &*(self.as_slice() as *const [f32]) }
    }
}

impl<'a> InterleavedBufferMut<'a> for Vec<f32> {
    fn buffer_mut(&mut self) -> &'a mut [f32] {
        // SAFETY: The lifetime 'a is tied to the underlying slice, which is valid.
        // The compiler incorrectly restricts the lifetime to that of `&mut self`.
        unsafe { &mut *(self.as_mut_slice() as *mut [f32]) }
    }
}

// -------------------------------------------------------------------------------------------------

/// A preallocated buffer slice with persistent start/end positions and helper functions.
#[derive(Clone, Debug)]
pub struct TempBuffer {
    buffer: Vec<f32>,
    start: usize,
    end: usize,
}

impl TempBuffer {
    /// Create a new empty buffer with the given capacity.
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: vec![0.0; capacity],
            start: 0,
            end: 0,
        }
    }

    /// Is there anything stored in the buffer?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.start >= self.end
    }
    /// Get temporary buffer's currently used length.
    #[inline]
    pub fn len(&self) -> usize {
        self.end - self.start
    }
    /// Get temporary buffers' total available capacity.
    #[inline]
    pub fn capacity(&self) -> usize {
        self.buffer.capacity()
    }

    /// Read-only access to the currently filled region.
    #[inline]
    pub fn get(&self) -> &'_ [f32] {
        &self.buffer[self.start..self.end]
    }
    /// Mutable access to the currently filled region.
    #[inline]
    pub fn get_mut(&mut self) -> &'_ mut [f32] {
        &mut self.buffer[self.start..self.end]
    }

    /// Set new filled range: usually will be done after writing to it via "get_mut".
    pub fn set_range(&mut self, start: usize, end: usize) {
        debug_assert!(start <= end);
        debug_assert!(end <= self.capacity());
        self.start = start;
        self.end = end;
    }
    /// Set range to cover the entire available capacity.
    pub fn reset_range(&mut self) {
        self.set_range(0, self.capacity());
    }

    /// Copy up to self.len().min(other.len()) to other. returns the sample len that got copied.
    pub fn copy_to(&self, other: &mut [f32]) -> usize {
        let copy_len = other.len().min(self.len());

        let other_slice = &mut other[..copy_len];
        let this_slice = &self.get()[..copy_len];
        copy_buffers(other_slice, this_slice);

        copy_len
    }
    /// Copy up to self.len().min(other.len()) from other. returns the sample len that got copied.
    pub fn copy_from(&mut self, other: &[f32]) -> usize {
        let copy_len = other.len().min(self.len());

        let this_slice = &mut self.get_mut()[..copy_len];
        let other_slice = &other[..copy_len];
        copy_buffers(this_slice, other_slice);

        copy_len
    }

    /// Mark the given amount in samples as used and remove it from the currently filled region.
    pub fn consume(&mut self, samples: usize) {
        self.start += samples;
        debug_assert!(self.start <= self.end);
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;

    #[test]
    fn planar_interleaved() {
        // mono
        let planar_mono = vec![vec![1.0, 2.0, 3.0, 4.0]];
        let interleaved_mono = vec![1.0, 2.0, 3.0, 4.0];
        let mut planar_mono_copy = planar_mono.clone();
        let mut interleaved_mono_copy = interleaved_mono.clone();

        planar_to_interleaved(&planar_mono, &mut interleaved_mono_copy);
        interleaved_to_planar(&interleaved_mono, &mut planar_mono_copy);
        assert_eq!(planar_mono, planar_mono_copy);
        assert_eq!(interleaved_mono, interleaved_mono_copy);

        // stereo
        let planar_stereo = vec![vec![1.0, 2.0, 3.0, 4.0], vec![4.0, 3.0, 2.0, 1.0]];
        let interleaved_stereo = vec![1.0, 4.0, 2.0, 3.0, 3.0, 2.0, 4.0, 1.0];
        let mut planar_stereo_copy = planar_stereo.clone();
        let mut interleaved_stereo_copy = interleaved_stereo.clone();

        planar_to_interleaved(&planar_stereo, &mut interleaved_stereo_copy);
        interleaved_to_planar(&interleaved_stereo, &mut planar_stereo_copy);
        assert_eq!(planar_stereo, planar_stereo_copy);
        assert_eq!(interleaved_stereo, interleaved_stereo_copy);

        // general
        let planar_general = vec![
            vec![1.0, 2.0, 3.0, 4.0],
            vec![4.0, 3.0, 2.0, 1.0],
            vec![2.0, 1.0, 4.0, 3.0],
        ];
        let interleaved_general = vec![1.0, 4.0, 2.0, 2.0, 3.0, 1.0, 3.0, 2.0, 4.0, 4.0, 1.0, 3.0];
        let mut planar_general_copy = planar_general.clone();
        let mut interleaved_general_copy = interleaved_general.clone();
        planar_to_interleaved(&planar_general, &mut interleaved_general_copy);
        interleaved_to_planar(&interleaved_general, &mut planar_general_copy);
        assert_eq!(planar_general, planar_general_copy);
        assert_eq!(interleaved_general, interleaved_general_copy);
    }

    #[test]
    fn clear_buffer_simd() {
        let mut buffer = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
        clear_buffer(&mut buffer);
        assert_eq!(
            buffer,
            vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
        );
    }

    #[test]
    fn scale_buffer_simd() {
        let mut buffer = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
        scale_buffer(&mut buffer, 2.0);
        assert_eq!(
            buffer,
            vec![2.0, 4.0, 6.0, 8.0, 10.0, 12.0, 14.0, 16.0, 18.0, 20.0, 22.0]
        );

        scale_buffer(&mut buffer, 0.5);
        assert_eq!(
            buffer,
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]
        );
    }

    #[test]
    fn add_buffers_simd() {
        let mut dest = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
        let source = vec![0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5];
        add_buffers(&mut dest, &source);
        assert_eq!(
            dest,
            vec![1.5, 3.0, 4.5, 6.0, 7.5, 9.0, 10.5, 12.0, 13.5, 15.0, 16.5]
        );
    }

    #[test]
    fn copy_buffers_simd() {
        let mut dest = vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0];
        let source = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0];
        copy_buffers(&mut dest, &source);
        assert_eq!(
            dest,
            vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0]
        );
    }

    #[test]
    fn max_abs_sample_simd() {
        let buffer = vec![
            0.1, -0.5, 0.3, -0.2, 0.15, -0.25, 0.35, -0.45, 0.05, -0.15, 0.25,
        ];
        let max = max_abs_sample(&buffer);
        assert_eq!(max, 0.5);

        let buffer: Vec<f32> = vec![];
        let max = max_abs_sample(&buffer);
        assert_eq!(max, 0.0);
    }

    #[test]
    fn buffer_channels_iter() {
        let mut data = vec![
            0.0_f32, 1.0, // frame 0: L, R
            10.0, 11.0, // frame 1: L, R
            20.0, 21.0, // frame 2: L, R
        ];
        let channels: Vec<Vec<f32>> = data.channels(2).map(|ch| ch.copied().collect()).collect();
        assert_eq!(channels, vec![vec![0.0, 10.0, 20.0], vec![1.0, 11.0, 21.0]]);

        for (i, ch) in data.channels_mut(2).enumerate() {
            if i == 0 {
                // left channel * 2
                for s in ch {
                    *s *= 2.0;
                }
            } else {
                // right channel * 3
                for s in ch {
                    *s *= 3.0;
                }
            }
        }
        assert_eq!(&data, &[0.0_f32, 3.0, 20.0, 33.0, 40.0, 63.0]);
    }

    #[test]
    fn buffer_frames_iter() {
        let mut data = vec![
            0.0_f32, 1.0, // frame 0
            10.0, 11.0, // frame 1
            20.0, 21.0, // frame 2
        ];

        let frames: Vec<Vec<f32>> = data.frames(2).map(|f| f.copied().collect()).collect();
        assert_eq!(
            frames,
            vec![vec![0.0, 1.0], vec![10.0, 11.0], vec![20.0, 21.0]]
        );

        for frame in data.as_mut_slice().frames_mut(2) {
            for s in frame {
                *s += 0.5;
            }
        }
        assert_eq!(&data, &[0.5_f32, 1.5, 10.5, 11.5, 20.5, 21.5]);
    }

    #[test]
    fn buffer_as_frames() {
        let mut data = vec![
            0.0_f32, 1.0, // frame 0: L, R
            10.0, 11.0, // frame 1: L, R
            20.0, 21.0, // frame 2: L, R
        ];
        let frames = data.as_frames::<2>();
        assert_eq!(frames, &[[0.0_f32, 1.0], [10.0, 11.0], [20.0, 21.0]][..]);

        let frames = data.as_frames_mut::<2>();
        for frame in frames.iter_mut() {
            frame[0] += 0.25;
            frame[1] += 0.75;
        }
        assert_eq!(&data, &[0.25_f32, 1.75, 10.25, 11.75, 20.25, 21.75]);
    }

    #[test]
    #[should_panic]
    fn buffer_as_frames_constraints() {
        let data = vec![
            0.0_f32, 1.0, // frame 0: L, R
            10.0, 11.0, // frame 1: L, R
            20.0, 21.0, // frame 2: L, R
        ];
        // 5 channels does not fit into 3*2 samples
        let _frames = data.as_frames::<5>();
    }
}
