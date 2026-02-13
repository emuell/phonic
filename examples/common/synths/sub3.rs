//! Substractive synth with 3 main and one sub oscillator, insprired by Novation's Bass Station.
//! To be wrapped into a [`FunDspGenerator`].
//!
//! - 4 oscillators (OSC 1, OSC 2, Sub OSC + Noise).
//! - Ring Modulation (OSC 1*2).
//! - 4 different OSC types (Sine, Triangle, Saw, Pulse)
//! - Moog Ladder filter with Drive and Key Tracking.
//! - 2 LFOs and 2 AHDSR Envelopes (Modulation & Amplitude).

use phonic::{
    four_cc::FourCC,
    fundsp::prelude32::*,
    generators::{LfoWaveform, ModulationConfig, ModulationSource, ModulationTarget},
    parameters::{EnumParameter, FloatParameter, IntegerParameter},
    utils::fundsp::{multi_osc, shared_ahdsr, var_buffer, SharedBuffer},
    Parameter, ParameterScaling,
};

use strum::VariantNames;

// -------------------------------------------------------------------------------------------------

// OSCILLATOR 1 PARAMETERS

pub const O1_RANGE: EnumParameter = EnumParameter::new(
    FourCC(*b"o1Rg"),
    "Osc1 Range",
    &["16'", "8'", "4'", "2'"],
    1,
);

pub const O1_WAVE: EnumParameter = EnumParameter::new(
    FourCC(*b"o1Wv"),
    "Osc1 Wave",
    &["Sine", "Triangle", "Sawtooth", "Pulse"],
    2,
);

pub const O1_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o1Cr"), "Osc1 Coarse", -12..=12, 0).with_unit("st");

pub const O1_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Fn"), "Osc1 Fine", -100.0..=100.0, 0.0).with_unit("cents");

pub const O1_PW: FloatParameter =
    FloatParameter::new(FourCC(*b"o1PW"), "Osc1 PulseWidth", 5.0..=95.0, 50.0).with_unit("%");

pub const O1_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Lv"), "Osc1 Level", 0.0..=1.0, 1.0);

// OSCILLATOR 2

pub const O2_RANGE: EnumParameter = EnumParameter::new(
    FourCC(*b"o2Rg"),
    "Osc2 Range",
    &["16'", "8'", "4'", "2'"],
    1,
);

pub const O2_WAVE: EnumParameter = EnumParameter::new(
    FourCC(*b"o2Wv"),
    "Osc2 Wave",
    &["Sine", "Triangle", "Sawtooth", "Pulse"],
    3,
);

pub const O2_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o2Cr"), "Osc2 Coarse", -12..=12, 0).with_unit("st");

pub const O2_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Fn"), "Osc2 Fine", -100.0..=100.0, 0.0).with_unit("cents");

pub const O2_PW: FloatParameter =
    FloatParameter::new(FourCC(*b"o2PW"), "Osc2 PulseWidth", 5.0..=95.0, 50.0).with_unit("%");

pub const O2_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Lv"), "Osc2 Level", 0.0..=1.0, 0.5);

// SUB OSCILLATOR

pub const SUB_OCTAVE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"suOc"), "Sub Octave", -2..=-1, -1);

pub const SUB_WAVE: EnumParameter = EnumParameter::new(
    FourCC(*b"suWv"),
    "Sub Wave",
    &["Sine", "Pulse", "Square"],
    0,
);

pub const SUB_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"suLv"), "Sub Level", 0.0..=1.0, 0.25);

// MIXER / OTHER

pub const NOISE_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"noLv"), "Noise Level", 0.0..=1.0, 0.0);

pub const RING_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"rmLv"), "RingMod Level", 0.0..=1.0, 0.0);

// FILTER

pub const FILTER_FREQ: FloatParameter =
    FloatParameter::new(FourCC(*b"flFr"), "Filter Freq", 20.0..=20000.0, 20000.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(3.0));

pub const FILTER_RES: FloatParameter =
    FloatParameter::new(FourCC(*b"flRs"), "Filter Res", 0.0..=1.0, 0.0);

pub const FILTER_DRIVE: FloatParameter =
    FloatParameter::new(FourCC(*b"flDr"), "Filter Drive", 0.0..=1.0, 0.0);

// AMP ENVELOPE

pub const AENV_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"aeAt"), "AmpEnv Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const AENV_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"aeHo"), "AmpEnv Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const AENV_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"aeDc"), "AmpEnv Decay", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const AENV_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"aeSu"), "AmpEnv Sustain", 0.0..=1.0, 1.0);
pub const AENV_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"aeRl"), "AmpEnv Release", 0.001..=5.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// MODULATION LFO

pub const LFO1_SPEED: FloatParameter =
    FloatParameter::new(FourCC(*b"l1Sp"), "LFO1 Speed", 0.01..=200.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(4.0));

pub const LFO1_WAVEFORM: EnumParameter = EnumParameter::new(
    FourCC(*b"l1Wv"),
    "LFO1 Waveform",
    LfoWaveform::VARIANTS,
    LfoWaveform::Sine as usize,
);

pub const LFO2_SPEED: FloatParameter =
    FloatParameter::new(FourCC(*b"l2Sp"), "LFO2 Speed", 0.01..=200.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(4.0));

pub const LFO2_WAVEFORM: EnumParameter = EnumParameter::new(
    FourCC(*b"l2Wv"),
    "LFO2 Waveform",
    LfoWaveform::VARIANTS,
    LfoWaveform::Sine as usize,
);

// MODULATION ENVELOPE

pub const MENV_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"meAt"), "ModEnv Attack", 0.001..=5.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const MENV_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"meHo"), "ModEnv Hold", 0.0..=5.0, 0.0).with_unit("s");
pub const MENV_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"meDc"), "ModEnv Decay", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const MENV_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"meSu"), "ModEnv Sustain", 0.0..=1.0, 0.5);
pub const MENV_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"meRl"), "ModEnv Release", 0.001..=5.0, 0.5)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// -------------------------------------------------------------------------------------------------

// Modulation Source IDs
pub const MOD_SRC_LFO1: FourCC = FourCC(*b"LFO1");
pub const MOD_SRC_LFO2: FourCC = FourCC(*b"LFO2");
pub const MOD_SRC_MOD_ENV: FourCC = FourCC(*b"MENV");
pub const MOD_SRC_VELOCITY: FourCC = FourCC(*b"MVEL");
pub const MOD_SRC_KEYTRACK: FourCC = FourCC(*b"MKEY");

// Modulation Target IDs (virtual parameters for modulation)
pub const MOD_TARGET_OSC1_PITCH: FourCC = FourCC(*b"mO1P");
pub const MOD_TARGET_OSC2_PITCH: FourCC = FourCC(*b"mO2P");
pub const MOD_TARGET_OSC1_PW: FourCC = FourCC(*b"mO1W");
pub const MOD_TARGET_OSC2_PW: FourCC = FourCC(*b"mO2W");
pub const MOD_TARGET_RING_LEVEL: FourCC = FourCC(*b"mRng");

// -------------------------------------------------------------------------------------------------

/// Exposes all automateable parameters. *excluding* modulation source parameters.
pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMS: [&'static dyn Parameter; 25] = [
        &O1_RANGE,
        &O1_WAVE,
        &O1_COARSE,
        &O1_FINE,
        &O1_PW,
        &O1_LEVEL,
        &O2_RANGE,
        &O2_WAVE,
        &O2_COARSE,
        &O2_FINE,
        &O2_PW,
        &O2_LEVEL,
        &SUB_OCTAVE,
        &SUB_WAVE,
        &SUB_LEVEL,
        &NOISE_LEVEL,
        &RING_LEVEL,
        &FILTER_FREQ,
        &FILTER_RES,
        &FILTER_DRIVE,
        &AENV_ATTACK,
        &AENV_HOLD,
        &AENV_DECAY,
        &AENV_SUSTAIN,
        &AENV_RELEASE,
    ];
    &ALL_PARAMS
}

// -------------------------------------------------------------------------------------------------

/// Returns the modulation configuration for the sub3 synth.
pub fn modulation_config() -> ModulationConfig {
    ModulationConfig {
        sources: vec![
            // LFO 1 for oscillator pitch modulation
            ModulationSource::Lfo {
                id: MOD_SRC_LFO1,
                name: "LFO 1",
                rate_param: LFO1_SPEED,
                waveform_param: LFO1_WAVEFORM,
            },
            // LFO 2 for filter cutoff modulation
            ModulationSource::Lfo {
                id: MOD_SRC_LFO2,
                name: "LFO 2",
                rate_param: LFO2_SPEED,
                waveform_param: LFO2_WAVEFORM,
            },
            // Modulation envelope for pitch and filter modulation
            ModulationSource::Envelope {
                id: MOD_SRC_MOD_ENV,
                name: "Mod Envelope",
                attack_param: MENV_ATTACK,
                hold_param: MENV_HOLD,
                decay_param: MENV_DECAY,
                sustain_param: MENV_SUSTAIN,
                release_param: MENV_RELEASE,
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
            ModulationTarget::new(MOD_TARGET_OSC1_PITCH, "OSC1 Pitch"),
            ModulationTarget::new(MOD_TARGET_OSC2_PITCH, "OSC2 Pitch"),
            ModulationTarget::new(MOD_TARGET_OSC1_PW, "OSC1 PulseWidth"),
            ModulationTarget::new(MOD_TARGET_OSC2_PW, "OSC2 PulseWidth"),
            ModulationTarget::new(MOD_TARGET_RING_LEVEL, "Ring Mod Level"),
            ModulationTarget::new(FILTER_FREQ.id(), FILTER_FREQ.name()),
            ModulationTarget::new(FILTER_RES.id(), FILTER_RES.name()),
            ModulationTarget::new(FILTER_DRIVE.id(), FILTER_DRIVE.name()),
        ],
    }
}

// -------------------------------------------------------------------------------------------------

/// Returns random parameter values.
/// Returns Vec of (parameter_id, normalized_value).
#[allow(unused)]
pub fn randomize() -> Vec<(FourCC, f32)> {
    let mut updates = Vec::new();
    for param in parameters() {
        let id = param.id();
        let value = if id == O1_RANGE.id() || id == O2_RANGE.id() {
            // 0=16', 1=8', 2=4', 3=2'
            // Weighted towards 8' (1) and 4' (2)
            let r = rand::random_range(0.0..1.0);
            let idx = if r < 0.1 {
                0
            } else if r < 0.5 {
                1
            } else if r < 0.9 {
                2
            } else {
                3
            };
            idx as f32 / 3.0
        } else if id == O1_WAVE.id() || id == O2_WAVE.id() {
            // 0=Sine, 1=Tri, 2=Saw, 3=Pulse
            let idx = rand::random_range(0..4);
            idx as f32 / 3.0
        } else if id == SUB_WAVE.id() {
            // 0=Sine, 1=Pulse, 2=Square
            let idx = rand::random_range(0..3);
            idx as f32 / 2.0
        } else if id == SUB_OCTAVE.id() {
            // -2 or -1
            if rand::random_range(0.0..1.0) < 0.5 {
                0.0
            } else {
                1.0
            }
        } else if id == O1_COARSE.id() || id == O2_COARSE.id() {
            // Mostly 0, sometimes -12, +12, +7
            let r = rand::random_range(0.0..1.0);
            let val = if r < 0.7 {
                0
            } else if r < 0.85 {
                -12
            } else if r < 0.95 {
                12
            } else {
                7
            };
            O1_COARSE.normalize_value(val)
        } else if id == O1_FINE.id() || id == O2_FINE.id() {
            // Slight detune
            let val = rand::random_range(-10.0..10.0);
            O1_FINE.normalize_value(val)
        } else if id == O1_PW.id() || id == O2_PW.id() {
            let val = rand::random_range(30.0..70.0);
            O1_PW.normalize_value(val)
        } else if id == O1_LEVEL.id() {
            rand::random_range(0.8..1.0)
        } else if id == O2_LEVEL.id() {
            rand::random_range(0.4..1.0)
        } else if id == SUB_LEVEL.id() {
            rand::random_range(0.0..0.6)
        } else if id == NOISE_LEVEL.id() {
            if rand::random_range(0.0..1.0) < 0.2 {
                rand::random_range(0.0..0.3)
            } else {
                0.0
            }
        } else if id == RING_LEVEL.id() {
            if rand::random_range(0.0..1.0) < 0.2 {
                rand::random_range(0.0..0.5)
            } else {
                0.0
            }
        } else if id == FILTER_FREQ.id() {
            rand::random_range(0.2..1.0)
        } else if id == FILTER_RES.id() {
            rand::random_range(0.0..0.7)
        } else if id == FILTER_DRIVE.id() {
            if rand::random_range(0.0..1.0) < 0.5 {
                0.0
            } else {
                rand::random_range(0.0..0.5)
            }
        } else if id == AENV_ATTACK.id() {
            if rand::random_range(0.0..1.0) < 0.5 {
                0.0
            } else {
                let val = rand::random_range(0.001..0.5);
                AENV_ATTACK.normalize_value(val)
            }
        } else if id == AENV_SUSTAIN.id() {
            rand::random_range(0.5..1.0)
        } else if id == AENV_RELEASE.id() {
            let val = rand::random_range(0.1..1.0);
            AENV_RELEASE.normalize_value(val)
        } else {
            rand::random_range(0.0..1.0)
        };

        updates.push((id, value));
    }

    for param in modulation_config().source_parameters() {
        let id = param.id();
        let value = if id == LFO1_SPEED.id() || id == LFO2_SPEED.id() {
            let val = rand::random_range(0.5..10.0);
            LFO1_SPEED.normalize_value(val)
        } else if id == LFO1_WAVEFORM.id() || id == LFO2_WAVEFORM.id() {
            // Random waveform selection
            let idx = rand::random_range(0..LfoWaveform::VARIANTS.len());
            idx as f32 / (LfoWaveform::VARIANTS.len() - 1) as f32
        } else if id == MENV_ATTACK.id() {
            let val = rand::random_range(0.001..0.5);
            MENV_ATTACK.normalize_value(val)
        } else if id == MENV_DECAY.id() || id == MENV_RELEASE.id() {
            let val = rand::random_range(0.05..1.5);
            MENV_DECAY.normalize_value(val)
        } else if id == MENV_SUSTAIN.id() {
            rand::random_range(0.3..1.0)
        } else {
            rand::random_range(0.0..=1.0)
        };
        updates.push((id, value));
    }

    updates
}

/// Returns random modulation routing connections.
/// Returns Vec of (source_id, target_id, amount, bipolar).
#[allow(unused)]
pub fn randomize_modulation() -> Vec<(FourCC, FourCC, f32, bool)> {
    let mut routes = Vec::new();

    // LFO1 -> OSC1 Pitch: 10% chance (vibrato)
    if rand::random_range(0.0..1.0) < 0.1 {
        routes.push((
            MOD_SRC_LFO1,
            MOD_TARGET_OSC1_PITCH,
            rand::random_range(0.2..0.5),
            true, // bipolar
        ));
    }

    // LFO1 -> OSC2 Pitch: 10% chance (vibrato)
    if rand::random_range(0.0..1.0) < 0.1 {
        routes.push((
            MOD_SRC_LFO1,
            MOD_TARGET_OSC2_PITCH,
            rand::random_range(0.2..0.5),
            true, // bipolar
        ));
    }

    // LFO1 -> OSC1 PW: 25% chance
    if rand::random_range(0.0..1.0) < 0.25 {
        routes.push((
            MOD_SRC_LFO1,
            MOD_TARGET_OSC1_PW,
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // LFO2 -> Filter Freq: 50% chance
    if rand::random_range(0.0..1.0) < 0.5 {
        routes.push((
            MOD_SRC_LFO2,
            FILTER_FREQ.id(),
            rand::random_range(0.3..0.7),
            true, // bipolar
        ));
    }

    // LFO2 -> Filter Res: 20% chance
    if rand::random_range(0.0..1.0) < 0.2 {
        routes.push((
            MOD_SRC_LFO2,
            FILTER_RES.id(),
            rand::random_range(0.3..0.6),
            true, // bipolar
        ));
    }

    // Mod Env -> OSC1 Pitch: 10% chance
    if rand::random_range(0.0..1.0) < 0.1 {
        routes.push((
            MOD_SRC_MOD_ENV,
            MOD_TARGET_OSC1_PITCH,
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Mod Env -> OSC2 Pitch: 10% chance
    if rand::random_range(0.0..1.0) < 0.1 {
        routes.push((
            MOD_SRC_MOD_ENV,
            MOD_TARGET_OSC2_PITCH,
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Mod Env -> Filter Freq: 60% chance (classic filter sweep)
    if rand::random_range(0.0..1.0) < 0.6 {
        routes.push((
            MOD_SRC_MOD_ENV,
            FILTER_FREQ.id(),
            rand::random_range(0.5..0.9),
            false, // unipolar
        ));
    }

    // Velocity -> Filter Freq: 35% chance (velocity-sensitive brightness)
    if rand::random_range(0.0..1.0) < 0.35 {
        routes.push((
            MOD_SRC_VELOCITY,
            FILTER_FREQ.id(),
            rand::random_range(0.4..0.7),
            false, // unipolar
        ));
    }

    // Velocity -> Filter Drive: 30% chance (velocity-sensitive aggression)
    if rand::random_range(0.0..1.0) < 0.3 {
        routes.push((
            MOD_SRC_VELOCITY,
            FILTER_DRIVE.id(),
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Keytracking -> Filter Freq: 40% chance (brighter for higher notes)
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_KEYTRACK,
            FILTER_FREQ.id(),
            rand::random_range(0.3..0.6),
            false, // unipolar
        ));
    }

    // Keytracking -> Ring Mod Level: 15% chance (more ring mod on high notes)
    if rand::random_range(0.0..1.0) < 0.15 {
        routes.push((
            MOD_SRC_KEYTRACK,
            MOD_TARGET_RING_LEVEL,
            rand::random_range(0.3..0.5),
            false, // unipolar
        ));
    }

    routes
}

// -------------------------------------------------------------------------------------------------

/// Returns a single synths funDSP voice as audio unit.
pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    vol: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
    modulation: &mut dyn FnMut(FourCC) -> SharedBuffer,
) -> Box<dyn AudioUnit> {
    // --- Get modulation buffers ---
    let osc1_pitch_mod_buffer = modulation(MOD_TARGET_OSC1_PITCH);
    let osc2_pitch_mod_buffer = modulation(MOD_TARGET_OSC2_PITCH);
    let osc1_pw_mod_buffer = modulation(MOD_TARGET_OSC1_PW);
    let osc2_pw_mod_buffer = modulation(MOD_TARGET_OSC2_PW);
    let ring_level_mod_buffer = modulation(MOD_TARGET_RING_LEVEL);
    let filter_freq_mod_buffer = modulation(FILTER_FREQ.id());
    let filter_res_mod_buffer = modulation(FILTER_RES.id());
    let filter_drive_mod_buffer = modulation(FILTER_DRIVE.id());

    // --- Oscillator 1 ---

    // Pitch Calculation
    // Range: 0=16'(0.5x), 1=8'(1.0x), 2=4'(2.0x), 3=2'(4.0x) -> 2^(val-1)
    let o1_range = var(&parameter(O1_RANGE.id())) >> shape_fn(|x| 2.0f32.powf(x.round() - 1.0));

    // Coarse/Fine
    let o1_semitones = var(&parameter(O1_COARSE.id())) + (var(&parameter(O1_FINE.id())) * 0.01);
    let o1_pitch_mult = o1_semitones >> shape_fn(|x| 2.0f32.powf(x / 12.0));

    // Modulation from matrix (±1 octave range = ±12 semitones)
    let o1_mod_octaves = var_buffer(&osc1_pitch_mod_buffer);
    let o1_mod_mult = o1_mod_octaves >> shape_fn(|x| 2.0f32.powf(x));

    let o1_freq = var(&freq) * o1_range * o1_pitch_mult * o1_mod_mult;

    // Waveform Generation
    // 0=Sine, 1=Tri, 2=Saw, 3=Pulse
    let o1_w_sel = var(&parameter(O1_WAVE.id()));
    // Apply pulse width modulation (unipolar 0..1, modulates ±20% around base)
    let o1_pw_mod = var_buffer(&osc1_pw_mod_buffer) * 0.2;
    let o1_pw_base = var(&parameter(O1_PW.id())) * 0.01; // 5..95 -> 0.05..0.95
    let o1_pw = (o1_pw_base + o1_pw_mod) >> clip_to(0.05, 0.95);
    let o1_sig = (o1_freq.clone() | o1_pw | o1_w_sel) >> multi_osc();
    let o1_out = o1_sig.clone() * var(&parameter(O1_LEVEL.id()));

    // --- Oscillator 2 ---

    // Pitch Calculation
    let o2_range = var(&parameter(O2_RANGE.id())) >> shape_fn(|x| 2.0f32.powf(x.round() - 1.0));
    let o2_semitones = var(&parameter(O2_COARSE.id())) + (var(&parameter(O2_FINE.id())) * 0.01);
    let o2_pitch_mult = o2_semitones >> shape_fn(|x| 2.0f32.powf(x / 12.0));

    // Modulation from matrix (±1 octave range = ±12 semitones)
    let o2_mod_octaves = var_buffer(&osc2_pitch_mod_buffer);
    let o2_mod_mult = o2_mod_octaves >> shape_fn(|x| 2.0f32.powf(x));

    let o2_freq = var(&freq) * o2_range * o2_pitch_mult * o2_mod_mult;

    // Waveform Generation
    let o2_w_sel = var(&parameter(O2_WAVE.id()));
    // Apply pulse width modulation (unipolar 0..1, modulates ±20% around base)
    let o2_pw_mod = var_buffer(&osc2_pw_mod_buffer) * 0.2;
    let o2_pw_base = var(&parameter(O2_PW.id())) * 0.01;
    let o2_pw = (o2_pw_base + o2_pw_mod) >> clip_to(0.05, 0.95);
    let o2_sig = (o2_freq.clone() | o2_pw | o2_w_sel) >> multi_osc();
    let o2_out = o2_sig.clone() * var(&parameter(O2_LEVEL.id()));

    // --- Sub Oscillator ---

    // Locked to Osc 1 freq.
    let sub_oct_mult = var(&parameter(SUB_OCTAVE.id())) >> shape_fn(|x| 2.0f32.powf(x.round()));
    let sub_freq = o1_freq.clone() * sub_oct_mult;

    // Waveform: 0=Sine, 1=Pulse(Narrow), 2=Square
    let sub_w_sel = var(&parameter(SUB_WAVE.id()));

    // Map sub_w_sel to MultiOsc inputs
    // Sel: 0->0 (Sin), 1->3 (Pulse), 2->3 (Pulse).
    let sub_sel = sub_w_sel.clone() >> shape_fn(|x| if x.round() == 0.0 { 0.0 } else { 3.0 });
    // PW: 1->0.25, 2->0.50. (0->don't care)
    let sub_pw = sub_w_sel.clone() >> shape_fn(|x| if x.round() == 1.0 { 0.25 } else { 0.50 });
    let sub_sig = (sub_freq.clone() | sub_pw | sub_sel) >> multi_osc();
    let sub_out = sub_sig * var(&parameter(SUB_LEVEL.id()));

    // --- Noise ---
    let noise_sig = white();
    let noise_out = noise_sig * var(&parameter(NOISE_LEVEL.id()));

    // --- Ring Mod ---
    // Osc 1 * Osc 2
    // Note: Using clones of the oscillator signals.
    let ring_sig = o1_sig.clone() * o2_sig.clone();
    // Apply ring mod level modulation (unipolar 0..1)
    let ring_level_mod = var_buffer(&ring_level_mod_buffer);
    let ring_level = var(&parameter(RING_LEVEL.id())) * (1.0 + ring_level_mod);
    let ring_out = ring_sig * ring_level;

    // --- Mixer Sum ---
    let mixed = o1_out + o2_out + sub_out + noise_out + ring_out;

    // --- Filter Section ---

    // Filter Frequency Modulation
    // Base Freq
    let fl_base = var(&parameter(FILTER_FREQ.id()));

    // Modulation from matrix (±4 octaves range)
    let fl_mod_octaves = var_buffer(&filter_freq_mod_buffer) * 4.0;
    let fl_mod_mult = fl_mod_octaves >> shape_fn(|x| 2.0f32.powf(x));

    let fl_cutoff = (fl_base * fl_mod_mult) >> clip_to(20.0, 20000.0);

    // Filter Resonance with modulation (unipolar 0..1)
    let fl_res_mod = var_buffer(&filter_res_mod_buffer);
    let fl_res = (var(&parameter(FILTER_RES.id())) * (1.0 + fl_res_mod)) >> clip_to(0.0, 1.0);

    // Filter Drive with modulation (unipolar 0..1)
    let fl_drive_mod = var_buffer(&filter_drive_mod_buffer);
    let fl_drive = (var(&parameter(FILTER_DRIVE.id())) * (1.0 + fl_drive_mod)) >> clip_to(0.0, 1.0);

    // Apply Drive (Soft Clip / Tanh)
    // Gain = 1.0 + Drive * 7.0 (range: 1x-8x)
    let drive_gain = 1.0 + fl_drive * 7.0;
    let driven_sig = (mixed * drive_gain) >> shape_fn(|x| x.tanh());

    let filtered = (driven_sig | fl_cutoff | fl_res) >> moog();

    // --- Amp Envelope ---
    let amp_env = shared_ahdsr(
        gate,
        parameter(AENV_ATTACK.id()),
        parameter(AENV_HOLD.id()),
        parameter(AENV_DECAY.id()),
        parameter(AENV_SUSTAIN.id()),
        parameter(AENV_RELEASE.id()),
    );

    // Lower global volume, assuming both oscillators are playing together fully
    let global_vol = 0.5;

    // Apply Amp Env, Volume and Panning
    let final_mix = ((filtered * amp_env * var(&vol) * global_vol) | var(&panning)) >> panner();

    Box::new(final_mix)
}
