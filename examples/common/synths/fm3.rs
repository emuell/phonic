//! FM synth with 3 oscillators, insprired by Fors algorithm free Pivot FM synth.
//! To be wrapped into a [`FunDspGenerator`].
//!
//! - 3 Operators (A, B, C) where A is the carrier.
//! - FM Algorithm: C->B->A and C->A (controlled by Blend).
//! - 3 AHDSR envelopes (one per operator).
//! - 1 LFO and 1 Aux Envelope for pitch, filter, and modulation depth control.
//! - Multi-mode Filter (LP, HP, BP) on the output.

use phonic::{
    four_cc::FourCC,
    fundsp::hacker32::*,
    generators::shared_ahdsr,
    parameters::{EnumParameter, FloatParameter},
    Error, GeneratorPlaybackHandle, Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

// Frequency parameters
pub const FREQ_A: FloatParameter =
    FloatParameter::new(FourCC(*b"frqA"), "A Frequency", 20.0..=20000.0, 440.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

pub const FREQ_B: FloatParameter =
    FloatParameter::new(FourCC(*b"frqB"), "B Frequency", 20.0..=20000.0, 880.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

pub const FREQ_C: FloatParameter =
    FloatParameter::new(FourCC(*b"frqC"), "C Frequency", 20.0..=20000.0, 10000.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

// Blend parameters
pub const BLEND: FloatParameter =
    FloatParameter::new(FourCC(*b"blnd"), "Blend C→A/B", 0.0..=1.0, 0.5);

// Modulation depth parameters
pub const DEPTH_B_TO_A: FloatParameter =
    FloatParameter::new(FourCC(*b"dpBA"), "Depth B→A", 0.0..=10.0, 5.0);
pub const DEPTH_C_TO_A: FloatParameter =
    FloatParameter::new(FourCC(*b"dpCA"), "Depth C→A", 0.0..=10.0, 3.0);
pub const DEPTH_C_TO_B: FloatParameter =
    FloatParameter::new(FourCC(*b"dpCB"), "Depth C→B", 0.0..=10.0, 2.0);

// LFO Parameters
pub const LFO_FREQ: FloatParameter =
    FloatParameter::new(FourCC(*b"lfoF"), "LFO Freq", 0.1..=20.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const MOD_LFO_PITCH: FloatParameter =
    FloatParameter::new(FourCC(*b"mlPt"), "LFO→Pitch", 0.0..=1.0, 0.0);

pub const MOD_LFO_CUTOFF: FloatParameter =
    FloatParameter::new(FourCC(*b"mlCt"), "LFO→Cutoff", -1.0..=1.0, 0.0);

// Aux Envelope Parameters
pub const AUX_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"axAt"), "Aux Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const AUX_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"axHo"), "Aux Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const AUX_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"axDc"), "Aux Decay", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const AUX_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"axSu"), "Aux Sustain", 0.0..=1.0, 0.7);
pub const AUX_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"axRl"), "Aux Release", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const MOD_ENV_CUTOFF: FloatParameter =
    FloatParameter::new(FourCC(*b"meCt"), "Env→Cutoff", -1.0..=1.0, 0.0);

pub const MOD_ENV_DEPTH_BA: FloatParameter =
    FloatParameter::new(FourCC(*b"meBA"), "Env→Depth B-A", -1.0..=1.0, 0.0);

// Filter parameters
pub const FILTER_CUTOFF: FloatParameter =
    FloatParameter::new(FourCC(*b"fCut"), "Filter Cutoff", 20.0..=20000.0, 20000.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

pub const FILTER_RES: FloatParameter =
    FloatParameter::new(FourCC(*b"fRes"), "Filter Res", 0.1..=10.0, 0.707);

pub const FILTER_TYPE: EnumParameter = EnumParameter::new(
    FourCC(*b"fTyp"),
    "Filter Type",
    &["Lowpass", "Highpass", "Bandpass"],
    0,
);

// Operator A envelope parameters
pub const ATTACK_A: FloatParameter =
    FloatParameter::new(FourCC(*b"atA_"), "A Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const HOLD_A: FloatParameter =
    FloatParameter::new(FourCC(*b"hoA_"), "A Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const DECAY_A: FloatParameter =
    FloatParameter::new(FourCC(*b"dcA_"), "A Decay", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const SUSTAIN_A: FloatParameter =
    FloatParameter::new(FourCC(*b"suA_"), "A Sustain", 0.0..=1.0, 0.7);
pub const RELEASE_A: FloatParameter =
    FloatParameter::new(FourCC(*b"rlA_"), "A Release", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// Operator B envelope parameters
pub const ATTACK_B: FloatParameter =
    FloatParameter::new(FourCC(*b"atB_"), "B Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const HOLD_B: FloatParameter =
    FloatParameter::new(FourCC(*b"hoB_"), "B Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const DECAY_B: FloatParameter =
    FloatParameter::new(FourCC(*b"dcB_"), "B Decay", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const SUSTAIN_B: FloatParameter =
    FloatParameter::new(FourCC(*b"suB_"), "B Sustain", 0.0..=1.0, 0.7);
pub const RELEASE_B: FloatParameter =
    FloatParameter::new(FourCC(*b"rlB_"), "B Release", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// Operator C envelope parameters
pub const ATTACK_C: FloatParameter =
    FloatParameter::new(FourCC(*b"atC_"), "C Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const HOLD_C: FloatParameter =
    FloatParameter::new(FourCC(*b"hoC_"), "C Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const DECAY_C: FloatParameter =
    FloatParameter::new(FourCC(*b"dcC_"), "C Decay", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const SUSTAIN_C: FloatParameter =
    FloatParameter::new(FourCC(*b"suC_"), "C Sustain", 0.0..=1.0, 0.7);
pub const RELEASE_C: FloatParameter =
    FloatParameter::new(FourCC(*b"rlC_"), "C Release", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// -------------------------------------------------------------------------------------------------

/// Exposes all automateable parameters.
pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMETERS: [&dyn Parameter; 35] = [
        &FREQ_A,
        &FREQ_B,
        &FREQ_C,
        &BLEND,
        &DEPTH_B_TO_A,
        &DEPTH_C_TO_A,
        &DEPTH_C_TO_B,
        &LFO_FREQ,
        &MOD_LFO_PITCH,
        &MOD_LFO_CUTOFF,
        &AUX_ATTACK,
        &AUX_HOLD,
        &AUX_DECAY,
        &AUX_SUSTAIN,
        &AUX_RELEASE,
        &MOD_ENV_CUTOFF,
        &MOD_ENV_DEPTH_BA,
        &FILTER_CUTOFF,
        &FILTER_RES,
        &FILTER_TYPE,
        &ATTACK_A,
        &HOLD_A,
        &DECAY_A,
        &SUSTAIN_A,
        &RELEASE_A,
        &ATTACK_B,
        &HOLD_B,
        &DECAY_B,
        &SUSTAIN_B,
        &RELEASE_B,
        &ATTACK_C,
        &HOLD_C,
        &DECAY_C,
        &SUSTAIN_C,
        &RELEASE_C,
    ];
    &ALL_PARAMETERS
}

// -------------------------------------------------------------------------------------------------

/// Randomize all parameters.
#[allow(unused)]
pub fn randomize(generator: &GeneratorPlaybackHandle) -> Result<(), Error> {
    for param in parameters().iter().filter(|p| p.id() != FREQ_A.id()) {
        generator.set_parameter_normalized(param.id(), rand::random_range(0.0..=1.0), None)?;
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------------

/// Create a new voice for a `FunDspGenerator`.
pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    volume: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
) -> Box<dyn AudioUnit> {
    // --- Modulators ---

    // LFO
    let lfo_freq = var(&parameter(LFO_FREQ.id()));
    let lfo = lfo_freq >> sine();

    // Aux Envelope
    let aux_env = shared_ahdsr(
        gate.clone(),
        parameter(AUX_ATTACK.id()),
        parameter(AUX_HOLD.id()),
        parameter(AUX_DECAY.id()),
        parameter(AUX_SUSTAIN.id()),
        parameter(AUX_RELEASE.id()),
    );

    // --- Pitch Modulation ---
    // LFO -> Pitch (Global)
    // Simple linear FM for vibrato: freq * (1 + lfo * amt * 0.06) approx +/- 1 semitone at max
    let pitch_mod = 1.0 + lfo.clone() * var(&parameter(MOD_LFO_PITCH.id())) * 0.06;

    // Get base frequencies from parameters (scaled by note frequency and pitch mod)
    let freq_a = var(&parameter(FREQ_A.id())) * var(&freq) * (1.0 / 440.0) * pitch_mod.clone();
    let freq_b = var(&parameter(FREQ_B.id())) * var(&freq) * (1.0 / 440.0) * pitch_mod.clone();
    let freq_c = var(&parameter(FREQ_C.id())) * var(&freq) * (1.0 / 440.0) * pitch_mod.clone();

    // --- Envelopes ---

    // Create AHDSR envelopes for each operator
    let env_a = shared_ahdsr(
        gate.clone(),
        parameter(ATTACK_A.id()),
        parameter(HOLD_A.id()),
        parameter(DECAY_A.id()),
        parameter(SUSTAIN_A.id()),
        parameter(RELEASE_A.id()),
    );

    let env_b = shared_ahdsr(
        gate.clone(),
        parameter(ATTACK_B.id()),
        parameter(HOLD_B.id()),
        parameter(DECAY_B.id()),
        parameter(SUSTAIN_B.id()),
        parameter(RELEASE_B.id()),
    );

    let env_c = shared_ahdsr(
        gate.clone(),
        parameter(ATTACK_C.id()),
        parameter(HOLD_C.id()),
        parameter(DECAY_C.id()),
        parameter(SUSTAIN_C.id()),
        parameter(RELEASE_C.id()),
    );

    // --- Operators ---

    // Operator C: simple sine wave with envelope
    let op_c = (freq_c >> sine()) * env_c;

    // Blend modulation: C→B (1-blend), C->A (blend)
    let blend = var(&parameter(BLEND.id()));

    // Operator B: frequency-modulated by C with envelope
    let op_b = ((freq_b.clone()
        + (op_c.clone()
            * var(&parameter(DEPTH_C_TO_B.id()))
            * (1.0 - blend.clone())
            * freq_b.clone()))
        >> sine())
        * env_b;

    // Operator A: frequency-modulated by both B and C with envelope
    // Modulation: Aux Env -> Depth B->A
    let depth_ba_base = var(&parameter(DEPTH_B_TO_A.id()));
    let depth_ba_mod = aux_env.clone() * var(&parameter(MOD_ENV_DEPTH_BA.id())) * 5.0;
    let depth_ba = depth_ba_base + depth_ba_mod;

    let op_a = ((freq_a.clone()
        + (op_b * depth_ba * freq_a.clone())
        + (op_c * var(&parameter(DEPTH_C_TO_A.id())) * blend * freq_a.clone()))
        >> sine())
        * env_a;

    // --- Filter ---

    let cutoff_base = var(&parameter(FILTER_CUTOFF.id()));
    let q = var(&parameter(FILTER_RES.id()));
    let filter_type = var(&parameter(FILTER_TYPE.id()));

    // Filter Modulation
    // LFO -> Cutoff (+/- 1000 Hz range)
    // Aux Env -> Cutoff (+/- 5000 Hz range)
    let cutoff_mod = cutoff_base
        + (lfo.clone() * var(&parameter(MOD_LFO_CUTOFF.id())) * 1000.0)
        + (aux_env.clone() * var(&parameter(MOD_ENV_CUTOFF.id())) * 5000.0);
    let cutoff = cutoff_mod >> clip_to(20.0, 20000.0);
    let filtered_op_a = (op_a | cutoff | q | filter_type) >> An(MultiFilter::new());

    // Apply Volume and Panning
    let final_mix = ((filtered_op_a * var(&volume) * 0.3) | var(&panning)) >> panner();

    Box::new(final_mix)
}

// -------------------------------------------------------------------------------------------------

/// Filter that switches between Lowpass, Highpass, and Bandpass
/// using a single SVF simulation.
///
/// Inputs:
/// 0: Audio
/// 1: Cutoff Frequency (Hz)
/// 2: Q Factor
/// 3: Filter Type (0=LP, 1=HP, 2=BP)
#[derive(Clone)]
struct MultiFilter {
    ic1eq: f32,
    ic2eq: f32,
    sample_rate: f32,
}

impl MultiFilter {
    fn new() -> Self {
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
            sample_rate: 44100.0,
        }
    }
}

impl AudioNode for MultiFilter {
    const ID: u64 = 0x434241;
    type Inputs = U4;
    type Outputs = U1;

    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let audio = input[0];
        let cutoff = input[1];
        let q = input[2];
        let type_sel = input[3];

        // Clamp cutoff to be safe below Nyquist
        let cutoff = cutoff.clamp(10.0, self.sample_rate * 0.49);
        let q = q.max(0.1);

        let g = (std::f32::consts::PI * cutoff / self.sample_rate).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;

        let v3 = audio - self.ic2eq;
        let v1 = a1 * self.ic1eq + a2 * v3;
        let v2 = self.ic2eq + a2 * self.ic1eq + a3 * v3;

        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;

        let lp = v2;
        let bp = v1;
        let hp = audio - k * v1 - v2;

        let sel = type_sel.round() as i32;
        let out = match sel {
            0 => lp,
            1 => hp,
            2 => bp,
            _ => lp,
        };

        [out].into()
    }
}
