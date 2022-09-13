// -------------------------------------------------------------------------------------------------

/// Copy the given planar buffer into an interleaved one.
/// The planar buffer's layout defines layout of the interleaved buffer (channel and frame count).
pub fn planar_to_interleaved(planar: &[Vec<f32>], interleaved: &mut [f32]) {
    let channel_count = planar.len();
    match channel_count {
        1 => {
            for (i, p) in interleaved.iter_mut().zip(planar[0].iter()) {
                *i = *p;
            }
        }
        2 => {
            for (index, (l, r)) in planar[0].iter().zip(planar[1].iter()).enumerate() {
                interleaved[index * 2] = *l;
                interleaved[index * 2 + 1] = *r;
            }
        }
        _ => {
            for (channel_index, channel_values) in planar.iter().enumerate() {
                for (frame_index, value) in channel_values.iter().enumerate() {
                    interleaved[frame_index * channel_count + channel_index] = *value;
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Copy the given interleaved buffer into a planar one.
/// The planar buffer's layout defines layout of the interleaved buffer (channel and frame count).
pub fn interleaved_to_planar(interleaved: &[f32], planar: &mut [Vec<f32>]) {
    let channel_count = planar.len();
    match channel_count {
        1 => {
            for (p, i) in planar[0].iter_mut().zip(interleaved) {
                *p = *i;
            }
        }
        2 => {
            let left = &mut planar[0];
            for (index, l) in left.iter_mut().enumerate() {
                *l = interleaved[index * 2];
            }
            let right = &mut planar[1];
            for (index, r) in right.iter_mut().enumerate() {
                *r = interleaved[index * 2 + 1];
            }
        }
        _ => {
            for (channel_index, channel_values) in planar.iter_mut().enumerate() {
                for (frame_index, value) in channel_values.iter_mut().enumerate() {
                    *value = interleaved[frame_index * channel_count + channel_index];
                }
            }
        }
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
