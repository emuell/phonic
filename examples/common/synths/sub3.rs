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
    parameters::{EnumParameter, FloatParameter, IntegerParameter},
    utils::fundsp::{fast_sine, multi_osc, shared_ahdsr},
    Parameter, ParameterScaling,
};

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

pub const O1_LFO1_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"o1L1"), "Osc1 LFO1 Depth", -63.0..=63.0, 0.0);

pub const O1_MENV_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"o1ME"), "Osc1 ModEnv Depth", -63.0..=63.0, 0.0);

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

pub const O2_LFO1_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"o2L1"), "Osc2 LFO1 Depth", -63.0..=63.0, 0.0);

pub const O2_MENV_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"o2ME"), "Osc2 ModEnv Depth", -63.0..=63.0, 0.0);

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

pub const FILTER_TRACK: FloatParameter = FloatParameter::new(
    FourCC(*b"flTr"),
    "Filter Tracking",
    0.0..=1.0,
    0.0, // 0=Full, 1=None
);

pub const FILTER_LFO2_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"flL2"), "Filter LFO2 Depth", -63.0..=63.0, 0.0);

pub const FILTER_MENV_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"flME"), "Filter ModEnv Depth", -63.0..=63.0, 0.0);

// MODULATION LFO

pub const LFO1_SPEED: FloatParameter =
    FloatParameter::new(FourCC(*b"l1Sp"), "LFO1 Speed", 0.1..=190.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const LFO2_SPEED: FloatParameter =
    FloatParameter::new(FourCC(*b"l2Sp"), "LFO2 Speed", 0.1..=190.0, 5.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

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

// -------------------------------------------------------------------------------------------------

pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMS: [&'static dyn Parameter; 39] = [
        &O1_RANGE,
        &O1_WAVE,
        &O1_COARSE,
        &O1_FINE,
        &O1_PW,
        &O1_LFO1_DEPTH,
        &O1_MENV_DEPTH,
        &O1_LEVEL,
        &O2_RANGE,
        &O2_WAVE,
        &O2_COARSE,
        &O2_FINE,
        &O2_PW,
        &O2_LFO1_DEPTH,
        &O2_MENV_DEPTH,
        &O2_LEVEL,
        &SUB_OCTAVE,
        &SUB_WAVE,
        &SUB_LEVEL,
        &NOISE_LEVEL,
        &RING_LEVEL,
        &FILTER_FREQ,
        &FILTER_RES,
        &FILTER_DRIVE,
        &FILTER_TRACK,
        &FILTER_LFO2_DEPTH,
        &FILTER_MENV_DEPTH,
        &LFO1_SPEED,
        &LFO2_SPEED,
        &MENV_ATTACK,
        &MENV_HOLD,
        &MENV_DECAY,
        &MENV_SUSTAIN,
        &MENV_RELEASE,
        &AENV_ATTACK,
        &AENV_HOLD,
        &AENV_DECAY,
        &AENV_SUSTAIN,
        &AENV_RELEASE,
    ];
    &ALL_PARAMS
}

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
        } else if (id == O1_LFO1_DEPTH.id() || id == O2_LFO1_DEPTH.id()) {
            if rand::random_range(0.0..1.0) < 0.2 {
                rand::random_range(0.0..1.0)
            } else {
                0.5
            }
        } else if (id == O1_MENV_DEPTH.id() || id == O2_MENV_DEPTH.id()) {
            if rand::random_range(0.0..1.0) < 0.3 {
                rand::random_range(0.0..1.0)
            } else {
                0.5
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
            rand::random_range(0.0..0.5)
        } else if id == FILTER_TRACK.id() {
            if rand::random_range(0.0..1.0) < 0.7 {
                0.0
            } else {
                rand::random_range(0.0..0.5)
            }
        } else if (id == FILTER_MENV_DEPTH.id() || id == FILTER_LFO2_DEPTH.id()) {
            if rand::random_range(0.0..1.0) < 0.5 {
                0.5
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
        } else if id == MENV_ATTACK.id() {
            let val = rand::random_range(0.001..0.5);
            MENV_ATTACK.normalize_value(val)
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
    updates
}

pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    vol: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
) -> Box<dyn AudioUnit> {
    // --- Modulation Sources ---

    // LFO 1
    let lfo1 = var(&parameter(LFO1_SPEED.id())) >> fast_sine();
    // LFO 2
    let lfo2 = var(&parameter(LFO2_SPEED.id())) >> fast_sine();

    // Mod Env
    let mod_env = shared_ahdsr(
        gate.clone(),
        parameter(MENV_ATTACK.id()),
        parameter(MENV_HOLD.id()),
        parameter(MENV_DECAY.id()),
        parameter(MENV_SUSTAIN.id()),
        parameter(MENV_RELEASE.id()),
    );

    // --- Oscillator 1 ---

    // Pitch Calculation
    // Range: 0=16'(0.5x), 1=8'(1.0x), 2=4'(2.0x), 3=2'(4.0x) -> 2^(val-1)
    let o1_range = var(&parameter(O1_RANGE.id())) >> shape_fn(|x| 2.0f32.powf(x.round() - 1.0));

    // Coarse/Fine
    let o1_semitones = var(&parameter(O1_COARSE.id())) + (var(&parameter(O1_FINE.id())) * 0.01);
    let o1_pitch_mult = o1_semitones >> shape_fn(|x| 2.0f32.powf(x / 12.0));

    // Modulation (LFO1 + ModEnv)
    // BSII Manual: LFO1 Depth -63 to +63.
    // Manual says: "32 = one octave". So 1 unit = 1/32 octave.
    let o1_lfo_mod = lfo1.clone() * var(&parameter(O1_LFO1_DEPTH.id())) * (1.0 / 32.0); // in octaves
    let o1_env_mod = mod_env.clone() * var(&parameter(O1_MENV_DEPTH.id())) * (1.0 / 8.0); // Arbitrary scaling
    let o1_mod_octaves = o1_lfo_mod + o1_env_mod;
    let o1_mod_mult = o1_mod_octaves >> shape_fn(|x| 2.0f32.powf(x));

    let o1_freq = var(&freq) * o1_range * o1_pitch_mult * o1_mod_mult;

    // Waveform Generation
    // 0=Sine, 1=Tri, 2=Saw, 3=Pulse
    let o1_w_sel = var(&parameter(O1_WAVE.id()));
    let o1_pw = var(&parameter(O1_PW.id())) * 0.01; // 5..95 -> 0.05..0.95
    let o1_sig = (o1_freq.clone() | o1_pw | o1_w_sel) >> multi_osc();
    let o1_out = o1_sig.clone() * var(&parameter(O1_LEVEL.id()));

    // --- Oscillator 2 ---

    // Pitch Calculation
    let o2_range = var(&parameter(O2_RANGE.id())) >> shape_fn(|x| 2.0f32.powf(x.round() - 1.0));
    let o2_semitones = var(&parameter(O2_COARSE.id())) + (var(&parameter(O2_FINE.id())) * 0.01);
    let o2_pitch_mult = o2_semitones >> shape_fn(|x| 2.0f32.powf(x / 12.0));

    let o2_lfo_mod = lfo1.clone() * var(&parameter(O2_LFO1_DEPTH.id())) * (1.0 / 32.0);
    let o2_env_mod = mod_env.clone() * var(&parameter(O2_MENV_DEPTH.id())) * (1.0 / 8.0);
    let o2_mod_mult = (o2_lfo_mod + o2_env_mod) >> shape_fn(|x| 2.0f32.powf(x));

    let o2_freq = var(&freq) * o2_range * o2_pitch_mult * o2_mod_mult;

    // Waveform Generation
    let o2_w_sel = var(&parameter(O2_WAVE.id()));
    let o2_pw = var(&parameter(O2_PW.id())) * 0.01;
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
    let ring_out = ring_sig * var(&parameter(RING_LEVEL.id()));

    // --- Mixer Sum ---
    let mixed = o1_out + o2_out + sub_out + noise_out + ring_out;

    // --- Filter Section ---

    // Filter Frequency Modulation
    // Base Freq
    let fl_base = var(&parameter(FILTER_FREQ.id()));

    // Tracking Amount 0..1
    let fl_track_amt = var(&parameter(FILTER_TRACK.id()));
    // Reference frequency (Middle C)
    let ref_freq = 261.626;
    // Ratio of current note freq to reference
    let note_ratio = var(&freq) * (1.0 / ref_freq);
    // Apply tracking amount to the ratio (in log domain, or just power)  ratio ^ tracking
    let track_mult = (fl_track_amt * (note_ratio >> shape_fn(|x| x.ln()))) >> shape_fn(|x| x.exp());

    // LFO 2 Mod
    // 16 = 1 octave. 1 unit = 1/16 octave.
    let fl_lfo_mod = lfo2 * var(&parameter(FILTER_LFO2_DEPTH.id())) * (1.0 / 16.0);

    // Mod Env Mod
    // Max depth = 8 octaves. Range -63..63.
    // 63 units = 8 octaves. 1 unit = 8/63 octaves.
    let fl_env_mod = mod_env.clone() * var(&parameter(FILTER_MENV_DEPTH.id())) * (8.0 / 63.0);

    let fl_mod_octaves = fl_lfo_mod + fl_env_mod;
    let fl_mod_mult = fl_mod_octaves >> shape_fn(|x| 2.0f32.powf(x));

    let fl_cutoff = (fl_base * track_mult * fl_mod_mult) >> clip_to(20.0, 20000.0);

    // Drive 0..1
    let fl_drive = var(&parameter(FILTER_DRIVE.id()));

    // Filter Node Inputs: Sig, Freq, Res
    let fl_res = var(&parameter(FILTER_RES.id()));

    // Apply Drive (Soft Clip / Tanh)
    // Gain = 1.0 + Drive
    let drive_gain = 1.0 + fl_drive;
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

    // Apply Amp Env and Volume and Panning
    let final_mix = ((filtered * amp_env * var(&vol)) | var(&panning)) >> panner();

    Box::new(final_mix)
}
