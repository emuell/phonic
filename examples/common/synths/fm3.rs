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
    fundsp::prelude32::*,
    generators::{LfoWaveform, ModulationConfig, ModulationSource, ModulationTarget},
    parameters::{EnumParameter, FloatParameter},
    utils::fundsp::{shared_ahdsr, var_buffer, SharedBuffer},
    Parameter, ParameterScaling,
};

use strum::VariantNames;

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
    FloatParameter::new(FourCC(*b"frqC"), "C Frequency", 20.0..=20000.0, 440.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

// Blend parameters
pub const BLEND: FloatParameter =
    FloatParameter::new(FourCC(*b"blnd"), "Blend C→A/B", 0.0..=1.0, 0.5);

// Modulation depth parameters
pub const DEPTH_B_TO_A: FloatParameter =
    FloatParameter::new(FourCC(*b"dpBA"), "Depth B→A", 0.0..=10.0, 1.5);
pub const DEPTH_C_TO_A: FloatParameter =
    FloatParameter::new(FourCC(*b"dpCA"), "Depth C→A", 0.0..=10.0, 1.0);
pub const DEPTH_C_TO_B: FloatParameter =
    FloatParameter::new(FourCC(*b"dpCB"), "Depth C→B", 0.0..=10.0, 0.5);

// LFO 1 Parameters
pub const LFO1_FREQ: FloatParameter =
    FloatParameter::new(FourCC(*b"lf1F"), "LFO1 Freq", 0.01..=20.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const LFO1_WAVEFORM: EnumParameter = EnumParameter::new(
    FourCC(*b"lf1W"),
    "LFO1 Waveform",
    LfoWaveform::VARIANTS,
    LfoWaveform::Sine as usize,
);

// LFO 2 Parameters
pub const LFO2_FREQ: FloatParameter =
    FloatParameter::new(FourCC(*b"lf2F"), "LFO2 Freq", 0.01..=20.0, 3.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const LFO2_WAVEFORM: EnumParameter = EnumParameter::new(
    FourCC(*b"lf2W"),
    "LFO2 Waveform",
    LfoWaveform::VARIANTS,
    LfoWaveform::Triangle as usize,
);

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

// Modulation Source IDs
pub const MOD_SRC_LFO1: FourCC = FourCC(*b"LFO1");
pub const MOD_SRC_LFO2: FourCC = FourCC(*b"LFO2");
pub const MOD_SRC_AUX_ENV: FourCC = FourCC(*b"AENV");
pub const MOD_SRC_VELOCITY: FourCC = FourCC(*b"MVEL");
pub const MOD_SRC_KEYTRACK: FourCC = FourCC(*b"MKEY");

// Modulation Target IDs (virtual parameters for modulation)
pub const MOD_TARGET_PITCH: FourCC = FourCC(*b"mPit");
pub const MOD_TARGET_DEPTH_BA: FourCC = FourCC(*b"mDBA");
pub const MOD_TARGET_DEPTH_CA: FourCC = FourCC(*b"mDCA");
pub const MOD_TARGET_DEPTH_CB: FourCC = FourCC(*b"mDCB");

// -------------------------------------------------------------------------------------------------

/// Exposes all automateable parameters. *excluding* modulation source parameters.
pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMETERS: [&dyn Parameter; 25] = [
        &FREQ_A,
        &FREQ_B,
        &FREQ_C,
        &BLEND,
        &DEPTH_B_TO_A,
        &DEPTH_C_TO_A,
        &DEPTH_C_TO_B,
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

/// Returns the modulation configuration for the FM3 synth.
pub fn modulation_config() -> ModulationConfig {
    ModulationConfig {
        sources: vec![
            // LFO 1 for pitch and filter modulation
            ModulationSource::Lfo {
                id: MOD_SRC_LFO1,
                name: "LFO 1",
                rate_param: LFO1_FREQ,
                waveform_param: LFO1_WAVEFORM,
            },
            // LFO 2 for filter and FM depth modulation
            ModulationSource::Lfo {
                id: MOD_SRC_LFO2,
                name: "LFO 2",
                rate_param: LFO2_FREQ,
                waveform_param: LFO2_WAVEFORM,
            },
            // Aux envelope for filter cutoff and FM depth modulation
            ModulationSource::Envelope {
                id: MOD_SRC_AUX_ENV,
                name: "Aux Envelope",
                attack_param: AUX_ATTACK,
                hold_param: AUX_HOLD,
                decay_param: AUX_DECAY,
                sustain_param: AUX_SUSTAIN,
                release_param: AUX_RELEASE,
            },
            // Velocity
            ModulationSource::Velocity {
                id: MOD_SRC_VELOCITY,
                name: "Velocity",
            },
            // Keytracking
            ModulationSource::Keytracking {
                id: MOD_SRC_KEYTRACK,
                name: "Keytracking",
            },
        ],
        targets: vec![
            ModulationTarget::new(FILTER_CUTOFF.id(), FILTER_CUTOFF.name()),
            ModulationTarget::new(FILTER_RES.id(), FILTER_RES.name()),
            ModulationTarget::new(MOD_TARGET_PITCH, "Pitch"),
            ModulationTarget::new(MOD_TARGET_DEPTH_BA, "Depth B→A"),
            ModulationTarget::new(MOD_TARGET_DEPTH_CA, "Depth C→A"),
            ModulationTarget::new(MOD_TARGET_DEPTH_CB, "Depth C→B"),
        ],
    }
}

// -------------------------------------------------------------------------------------------------

/// Randomize all parameters.
#[allow(unused)]
pub fn randomize() -> Vec<(FourCC, f32)> {
    let mut updates = Vec::new();
    // Pre-select filter type: 0=Lowpass, 1=Highpass, 2=Bandpass
    let filter_type_index = rand::random_range(0..3);

    for param in parameters() {
        let id = param.id();
        let value = if id == FREQ_A.id() {
            FREQ_A.normalize_value(FREQ_A.default_value())
        } else if id == FREQ_B.id() {
            let ratios = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];
            let ratio = ratios[rand::random_range(0..ratios.len())];
            let detune = rand::random_range(0.99..=1.01);
            FREQ_B.normalize_value(440.0 * ratio * detune)
        } else if id == FREQ_C.id() {
            let ratios = [0.5, 1.0, 1.5, 2.0, 3.0, 4.0, 5.0, 6.0, 8.0];
            let ratio = ratios[rand::random_range(0..ratios.len())];
            let detune = rand::random_range(0.99..=1.01);
            FREQ_C.normalize_value(440.0 * ratio * detune)
        } else if id == ATTACK_A.id() {
            if rand::random_range(0.0..1.0) < 0.5 {
                0.0
            } else {
                rand::random_range(0.0..=1.0)
            }
        } else if id == DEPTH_B_TO_A.id() || id == DEPTH_C_TO_A.id() || id == DEPTH_C_TO_B.id() {
            // Limit FM depth to 40% (0.0 to 4.0) to avoid extreme aliasing
            rand::random_range(0.0..=0.4)
        } else if id == FILTER_TYPE.id() {
            FILTER_TYPE.normalize_value(FILTER_TYPE.values()[filter_type_index])
        } else if id == FILTER_CUTOFF.id() {
            // Set cutoff based on selected filter type
            match filter_type_index {
                0 => rand::random_range(0.5..=1.0),
                1 => rand::random_range(0.0..=0.5),
                _ => rand::random_range(0.15..=0.35),
            }
        } else {
            rand::random_range(0.0..=1.0)
        };

        updates.push((id, value));
    }

    for param in modulation_config().source_parameters() {
        let id = param.id();
        let value = if id == LFO1_FREQ.id() || id == LFO2_FREQ.id() {
            let val = rand::random_range(0.5..10.0);
            LFO1_FREQ.normalize_value(val)
        } else if id == LFO1_WAVEFORM.id() || id == LFO2_WAVEFORM.id() {
            // Random waveform selection
            let idx = rand::random_range(0..LfoWaveform::VARIANTS.len());
            idx as f32 / (LfoWaveform::VARIANTS.len() - 1) as f32
        } else if id == AUX_ATTACK.id() {
            let val = rand::random_range(0.001..0.5);
            AUX_ATTACK.normalize_value(val)
        } else if id == AUX_DECAY.id() || id == AUX_RELEASE.id() {
            let val = rand::random_range(0.05..1.5);
            AUX_DECAY.normalize_value(val)
        } else if id == AUX_SUSTAIN.id() {
            rand::random_range(0.3..1.0)
        } else {
            rand::random_range(0.0..=1.0)
        };
        updates.push((id, value));
    }

    updates
}

// -------------------------------------------------------------------------------------------------

/// Returns random modulation routing connections.
/// Returns Vec of (source_id, target_id, amount, bipolar).
#[allow(unused)]
pub fn randomize_modulation() -> Vec<(FourCC, FourCC, f32, bool)> {
    let mut routes = Vec::new();

    // LFO1 -> Pitch (vibrato): 15% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.15 {
        routes.push((
            MOD_SRC_LFO1,
            MOD_TARGET_PITCH,
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // LFO1 -> Filter Cutoff: 40% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_LFO1,
            FILTER_CUTOFF.id(),
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // LFO2 -> Filter Cutoff: 40% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_LFO2,
            FILTER_CUTOFF.id(),
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // LFO2 -> Filter Res: 25% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.25 {
        routes.push((
            MOD_SRC_LFO2,
            FILTER_RES.id(),
            rand::random_range(0.3..0.6),
            true, // bipolar
        ));
    }

    // LFO2 -> FM Depth B→A: 20% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.2 {
        routes.push((
            MOD_SRC_LFO2,
            MOD_TARGET_DEPTH_BA,
            rand::random_range(0.3..0.6),
            true, // bipolar
        ));
    }

    // Aux Env -> Filter Cutoff: 60% chance, unipolar (classic filter sweep)
    if rand::random_range(0.0..1.0) < 0.6 {
        routes.push((
            MOD_SRC_AUX_ENV,
            FILTER_CUTOFF.id(),
            rand::random_range(0.4..0.9),
            false, // unipolar
        ));
    }

    // Aux Env -> FM Depth B→A: 40% chance, bipolar (evolving timbre)
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_AUX_ENV,
            MOD_TARGET_DEPTH_BA,
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // Aux Env -> FM Depth C→A: 40% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_AUX_ENV,
            MOD_TARGET_DEPTH_CA,
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // Aux Env -> FM Depth C→B: 40% chance, bipolar
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_AUX_ENV,
            MOD_TARGET_DEPTH_CB,
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // Velocity -> Filter Cutoff: 35% chance (velocity-sensitive brightness)
    if rand::random_range(0.0..1.0) < 0.35 {
        routes.push((
            MOD_SRC_VELOCITY,
            FILTER_CUTOFF.id(),
            rand::random_range(0.4..0.7),
            false, // unipolar
        ));
    }

    // Velocity -> FM Depth B→A: 30% chance (velocity-sensitive complexity)
    if rand::random_range(0.0..1.0) < 0.3 {
        routes.push((
            MOD_SRC_VELOCITY,
            MOD_TARGET_DEPTH_BA,
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Keytracking -> Filter Cutoff: 40% chance (brighter for higher notes)
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_KEYTRACK,
            FILTER_CUTOFF.id(),
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Keytracking -> Filter Res: 20% chance (more resonance on high notes)
    if rand::random_range(0.0..1.0) < 0.2 {
        routes.push((
            MOD_SRC_KEYTRACK,
            FILTER_RES.id(),
            rand::random_range(0.3..0.5),
            false, // unipolar
        ));
    }

    routes
}

// -------------------------------------------------------------------------------------------------

/// Create a new voice for a `FunDspGenerator`.
pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    volume: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
    modulation: &mut dyn FnMut(FourCC) -> SharedBuffer,
) -> Box<dyn AudioUnit> {
    // --- Get modulation buffers ---
    let pitch_mod_buffer = modulation(MOD_TARGET_PITCH);
    let cutoff_mod_buffer = modulation(FILTER_CUTOFF.id());
    let filter_res_mod_buffer = modulation(FILTER_RES.id());
    let depth_ba_mod_buffer = modulation(MOD_TARGET_DEPTH_BA);
    let depth_ca_mod_buffer = modulation(MOD_TARGET_DEPTH_CA);
    let depth_cb_mod_buffer = modulation(MOD_TARGET_DEPTH_CB);

    // --- Pitch Modulation ---
    // Modulation from matrix: ±0.06 (approx ±1 semitone range)
    let pitch_mod = 1.0 + var_buffer(&pitch_mod_buffer) * 0.06;

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
    // Modulation: Matrix -> Depth C->B (±5.0 range)
    let depth_cb_base = var(&parameter(DEPTH_C_TO_B.id()));
    let depth_cb_mod = var_buffer(&depth_cb_mod_buffer) * 5.0;
    let depth_cb = depth_cb_base + depth_cb_mod;

    let op_b = ((freq_b.clone()
        + (op_c.clone() * depth_cb * (1.0 - blend.clone()) * freq_b.clone()))
        >> sine())
        * env_b;

    // Operator A: frequency-modulated by both B and C with modulation
    // Modulation: Matrix -> Depth B->A (±5.0 range)
    let depth_ba_base = var(&parameter(DEPTH_B_TO_A.id()));
    let depth_ba_mod = var_buffer(&depth_ba_mod_buffer) * 5.0;
    let depth_ba = depth_ba_base + depth_ba_mod;

    // Modulation: Matrix -> Depth C->A (±5.0 range)
    let depth_ca_base = var(&parameter(DEPTH_C_TO_A.id()));
    let depth_ca_mod = var_buffer(&depth_ca_mod_buffer) * 5.0;
    let depth_ca = depth_ca_base + depth_ca_mod;

    let op_a = ((freq_a.clone()
        + (op_b * depth_ba * freq_a.clone())
        + (op_c * depth_ca * blend * freq_a.clone()))
        >> sine())
        * env_a;

    // --- Filter ---

    let cutoff_base = var(&parameter(FILTER_CUTOFF.id()));
    let filter_type = var(&parameter(FILTER_TYPE.id()));

    // Filter Cutoff Modulation from matrix (±5000 Hz range)
    let cutoff_mod = cutoff_base + var_buffer(&cutoff_mod_buffer) * 5000.0;
    let cutoff = cutoff_mod >> clip_to(20.0, 20000.0);

    // Filter Resonance Modulation (unipolar 0..1, modulates ±5.0 around base)
    let q_base = var(&parameter(FILTER_RES.id()));
    let q_mod = var_buffer(&filter_res_mod_buffer) * 5.0;
    let q = (q_base + q_mod) >> clip_to(0.1, 10.0);

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
