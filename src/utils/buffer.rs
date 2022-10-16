// -------------------------------------------------------------------------------------------------

/// Copy the given planar buffer into an interleaved one.
/// The planar buffer's layout defines layout of the interleaved buffer (channel and frame count).
/// The interleaved buffer must be large enough to fit the planar buffer.
pub fn planar_to_interleaved(planar: &[Vec<f32>], interleaved: &mut [f32]) {
    let channel_count = planar.len();
    let frame_count = planar[0].len();
    debug_assert!(
        interleaved.len() >= frame_count * channel_count,
        "Buffer size mismatch"
    );
    match channel_count {
        1 => {
            for (i, p) in interleaved.iter_mut().zip(planar[0].iter()) {
                *i = *p;
            }
        }
        2 => {
            for (i, (l, r)) in interleaved
                .chunks_mut(2)
                .zip(planar[0].iter().zip(planar[1].iter()))
            {
                unsafe {
                    *i.get_unchecked_mut(0) = *l;
                    *i.get_unchecked_mut(1) = *r;
                }
            }
        }
        _ => {
            for (channel_index, channel) in planar.iter().enumerate() {
                for (p, i) in channel
                    .iter()
                    .zip(interleaved.chunks_exact_mut(channel_count))
                {
                    unsafe {
                        *i.get_unchecked_mut(channel_index) = *p;
                    }
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
            for (p, i) in planar[0].iter_mut().zip(interleaved) {
                *p = *i;
            }
        }
        2 => {
            let left = &mut planar[0];
            for (p, i) in left.iter_mut().zip(interleaved.chunks_exact(2)) {
                unsafe {
                    *p = *i.get_unchecked(0);
                }
            }
            let right = &mut planar[1];
            for (p, i) in right.iter_mut().zip(interleaved.chunks_exact(2)) {
                unsafe {
                    *p = *i.get_unchecked(1);
                }
            }
        }
        _ => {
            for (channel_index, channel) in planar.iter_mut().enumerate() {
                for (p, i) in channel
                    .iter_mut()
                    .zip(interleaved.chunks_exact(channel_count))
                {
                    unsafe {
                        *p = *i.get_unchecked(channel_index);
                    }
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// A preallocated buffer with persistent start/end positions and helper functions,
/// which are useful for temporary interleaved sample buffers.
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
    pub fn get(&self) -> &[f32] {
        &self.buffer[self.start..self.end]
    }
    /// Mutable access to the currently filled region.
    #[inline]
    pub fn get_mut(&mut self) -> &mut [f32] {
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
        other_slice.copy_from_slice(this_slice);

        copy_len
    }
    /// Copy up to self.len().min(other.len()) from other. returns the sample len that got copied.
    pub fn copy_from(&mut self, other: &mut [f32]) -> usize {
        let copy_len = other.len().min(self.len());

        let other_slice = &mut other[..copy_len];
        let this_slice = &mut self.get_mut()[..copy_len];
        this_slice.copy_from_slice(other_slice);

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
    use super::*;
    use std::vec;

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
}
