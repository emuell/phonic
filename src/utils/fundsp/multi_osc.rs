use std::{f32, sync::Arc};

use fundsp::{
    audionode::AudioNode,
    buffer::{BufferMut, BufferRef},
    combinator::An,
    full_simd_items,
    typenum::{U1, U3},
    wavetable::{saw_table, triangle_table, Wavetable},
    F32x, Frame, SIMD_N, SIMD_S,
};

// -------------------------------------------------------------------------------------------------

/// Oscillator that switches between Sine, Triangle, Saw, and Pulse
/// without calculating all waveforms simultaneously.
///
/// Inputs:
/// 0: Frequency (Hz)
/// 1: Pulse Width (0.0 - 1.0)
/// 2: Waveform Selection (0=Sin, 1=Tri, 2=Saw, 3=Pulse)
#[derive(Clone)]
pub struct MultiOsc {
    saw: Arc<Wavetable>,
    tri: Arc<Wavetable>,
    phase: f32,
    sample_rate: f32,
    sample_duration: f32,
    saw_hint: usize,
    tri_hint: usize,
}

impl MultiOsc {
    pub fn new() -> Self {
        Self {
            saw: saw_table(),
            tri: triangle_table(),
            phase: 0.0,
            sample_rate: 44100.0,
            sample_duration: 1.0 / 44100.0,
            saw_hint: 0,
            tri_hint: 0,
        }
    }
}

impl Default for MultiOsc {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioNode for MultiOsc {
    const ID: u64 = 0x5E1EC7;
    type Inputs = U3;
    type Outputs = U1;

    fn reset(&mut self) {
        self.phase = 0.0;
        self.saw_hint = 0;
        self.tri_hint = 0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
        self.sample_duration = 1.0 / self.sample_rate;
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let freq = input[0];
        let pw = input[1];
        let sel = input[2];

        let delta = freq * self.sample_duration;
        self.phase += delta;
        self.phase -= self.phase.floor();

        // Selection: 0=Sin, 1=Tri, 2=Saw, 3=Pulse
        let sel_i = sel.round() as i32;

        let out = match sel_i {
            0 => (self.phase * std::f32::consts::TAU).sin(),
            1 => {
                let (v, h) = self.tri.read(self.tri_hint, freq.abs(), self.phase);
                self.tri_hint = h;
                v
            }
            2 => {
                let (v, h) = self.saw.read(self.saw_hint, freq.abs(), self.phase);
                self.saw_hint = h;
                v
            }
            3 => {
                let (v1, h1) = self.saw.read(self.saw_hint, freq.abs(), self.phase);

                let mut p2 = self.phase + pw;
                p2 -= p2.floor();
                let (v2, h2) = self.saw.read(h1, freq.abs(), p2);

                self.saw_hint = h2;
                v1 - v2
            }
            _ => 0.0,
        };
        [out].into()
    }

    fn process(&mut self, size: usize, input: &BufferRef, output: &mut BufferMut) {
        // Optimization: make waveform selection constant for the block, take first value only.
        let sel = input.at_f32(2, 0);
        let sel_i = sel.round() as i32;

        let mut phase = self.phase;
        for i in 0..full_simd_items(size) {
            // Generate phase for this SIMD chunk
            let phase_elements: [f32; SIMD_N] = core::array::from_fn(|j| {
                let p = phase;
                let f = input.at_f32(0, (i << SIMD_S) + j);
                phase += f * self.sample_duration;
                p
            });

            // Process each element in the chunk
            let out_elements: [f32; SIMD_N] = match sel_i {
                0 => core::array::from_fn(|j| {
                    let p = phase_elements[j];
                    let p_wrapped = p - p.floor();
                    (p_wrapped * std::f32::consts::TAU).sin()
                }),
                1 => core::array::from_fn(|j| {
                    let p = phase_elements[j];
                    let p_wrapped = p - p.floor();
                    let idx = (i << SIMD_S) + j;
                    let f = input.at_f32(0, idx);
                    let (v, h) = self.tri.read(self.tri_hint, f.abs(), p_wrapped);
                    self.tri_hint = h;
                    v
                }),
                2 => core::array::from_fn(|j| {
                    let p = phase_elements[j];
                    let p_wrapped = p - p.floor();
                    let idx = (i << SIMD_S) + j;
                    let f = input.at_f32(0, idx);
                    let (v, h) = self.saw.read(self.saw_hint, f.abs(), p_wrapped);
                    self.saw_hint = h;
                    v
                }),
                3 => core::array::from_fn(|j| {
                    let p = phase_elements[j];
                    let p_wrapped = p - p.floor();
                    let idx = (i << SIMD_S) + j;
                    let f = input.at_f32(0, idx);
                    let pw = input.at_f32(1, idx);

                    let (v1, h1) = self.saw.read(self.saw_hint, f.abs(), p_wrapped);
                    let mut p2 = p_wrapped + pw;
                    p2 -= p2.floor();
                    let (v2, h2) = self.saw.read(h1, f.abs(), p2);
                    self.saw_hint = h2;
                    v1 - v2
                }),
                _ => [0.0; SIMD_N],
            };

            output.set(0, i, F32x::new(out_elements));
        }

        self.phase = phase - phase.floor();
        self.process_remainder(size, input, output);
    }
}

// -------------------------------------------------------------------------------------------------

/// Create a variable oscillator with waveform selection as input.
///
/// Inputs:
/// 0: Frequency (Hz)
/// 1: Pulse Width (0.0 - 1.0)
/// 2: Waveform Selection (0=Sin, 1=Tri, 2=Saw, 3=Pulse)
pub fn multi_osc() -> An<MultiOsc> {
    An(MultiOsc::new())
}
