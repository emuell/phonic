//! FunDSP AudioNode for a fast sine wave oscillator.

use std::f32;

use fundsp::{
    audionode::AudioNode,
    combinator::An,
    convert,
    math::rnd1,
    signal::{Routing, SignalFrame},
    typenum, Frame, DEFAULT_SR,
};

// -------------------------------------------------------------------------------------------------

/// See https://web.archive.org/web/20171228230531/http://forum.devmaster.net/t/fast-and-accurate-sine-cosine/9648
/// x must be in range [-PI to PI]
fn sine_approx(x: f32) -> f32 {
    debug_assert!((-f32::consts::PI..=f32::consts::PI).contains(&x));

    const B: f32 = 4.0 / f32::consts::PI;
    const C: f32 = -4.0 / (f32::consts::PI * f32::consts::PI);
    const P: f32 = 0.225;

    let y = B * x + C * x * x.abs();
    P * (y * y.abs() - y) + y
}

// -------------------------------------------------------------------------------------------------

/// Sine oscillator using a fast sine approximation. Precise enough to be used as LFO.
/// - Input 0: frequency in Hz.
/// - Output 0: sine wave.
#[derive(Default, Clone)]
pub struct FastSine {
    phase: f32,
    sample_duration: f32,
    hash: u64,
    initial_phase: Option<f32>,
}

impl FastSine {
    /// Create sine oscillator.
    pub fn new() -> Self {
        let mut sine = FastSine::default();
        sine.reset();
        sine.set_sample_rate(DEFAULT_SR);
        sine
    }
    /// Create sine oscillator with initial phase in 0...1.
    pub fn with_phase(initial_phase: f32) -> Self {
        let mut sine = Self {
            phase: 0.0,
            sample_duration: 0.0,
            hash: 0,
            initial_phase: Some(initial_phase),
        };
        sine.reset();
        sine.set_sample_rate(DEFAULT_SR);
        sine
    }
}

impl AudioNode for FastSine {
    const ID: u64 = 2134;
    type Inputs = typenum::U1;
    type Outputs = typenum::U1;

    fn reset(&mut self) {
        self.phase = match self.initial_phase {
            Some(phase) => phase,
            None => convert(rnd1(self.hash)),
        };
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_duration = convert(1.0 / sample_rate);
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let phase = self.phase;
        self.phase += input[0] * self.sample_duration;
        self.phase -= self.phase.floor();
        [sine_approx(phase * f32::consts::TAU - f32::consts::PI)].into()
    }

    fn set_hash(&mut self, hash: u64) {
        self.hash = hash;
        self.reset();
    }

    fn route(&mut self, input: &SignalFrame, _frequency: f64) -> SignalFrame {
        Routing::Arbitrary(0.0).route(input, self.outputs())
    }
}

// -------------------------------------------------------------------------------------------------

/// Create an fast sine approximation, suitable as e.g. LFO.
pub fn fast_sine() -> An<FastSine> {
    An(FastSine::new())
}
