//! Simple Fundsp organ synth with 6 drawbars, percussion, click and vibrato.
//! To be wrapped into a [`FunDspGenerator`].

use phonic::{
    four_cc::FourCC,
    fundsp::prelude32::*,
    parameters::{EnumParameter, FloatParameter},
    utils::fundsp::shared_ahdsr,
    Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

// Drawbars (Volumes)
pub const DRAWBAR_16: FloatParameter =
    FloatParameter::new(FourCC(*b"db16"), "Drawbar 16'", 0.0..=1.0, 0.5);
pub const DRAWBAR_8: FloatParameter =
    FloatParameter::new(FourCC(*b"db08"), "Drawbar 8'", 0.0..=1.0, 1.0);
pub const DRAWBAR_5_1_3: FloatParameter =
    FloatParameter::new(FourCC(*b"db53"), "Drawbar 5 1/3'", 0.0..=1.0, 0.0);
pub const DRAWBAR_4: FloatParameter =
    FloatParameter::new(FourCC(*b"db04"), "Drawbar 4'", 0.0..=1.0, 0.5);
pub const DRAWBAR_2_2_3: FloatParameter =
    FloatParameter::new(FourCC(*b"db23"), "Drawbar 2 2/3'", 0.0..=1.0, 0.0);
pub const DRAWBAR_2: FloatParameter =
    FloatParameter::new(FourCC(*b"db02"), "Drawbar 2'", 0.0..=1.0, 0.2);

// Fine Tuning (Cents)
pub const TUNE_16: FloatParameter =
    FloatParameter::new(FourCC(*b"tn16"), "Tune 16'", -50.0..=50.0, 0.0).with_unit("cents");
pub const TUNE_5_1_3: FloatParameter =
    FloatParameter::new(FourCC(*b"tn53"), "Tune 5 1/3'", -50.0..=50.0, 0.0).with_unit("cents");
pub const TUNE_4: FloatParameter =
    FloatParameter::new(FourCC(*b"tn04"), "Tune 4'", -50.0..=50.0, 0.0).with_unit("cents");
pub const TUNE_2_2_3: FloatParameter =
    FloatParameter::new(FourCC(*b"tn23"), "Tune 2 2/3'", -50.0..=50.0, 0.0).with_unit("cents");
pub const TUNE_2: FloatParameter =
    FloatParameter::new(FourCC(*b"tn02"), "Tune 2'", -50.0..=50.0, 0.0).with_unit("cents");

// Envelope
pub const ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"attk"), "Attack", 0.001..=2.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"rels"), "Release", 0.001..=2.0, 1.0)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// Vibrato
pub const VIB_RATE: FloatParameter =
    FloatParameter::new(FourCC(*b"vRat"), "Vibrato Rate", 0.1..=10.0, 6.0).with_unit("Hz");
pub const VIB_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"vDep"), "Vibrato Depth", 0.0..=1.0, 0.0);

// Percussion
pub const PERC_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"pLvl"), "Perc Level", 0.0..=1.0, 0.0);
pub const PERC_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"pDec"), "Perc Decay", 0.01..=1.0, 0.2).with_unit("s");
pub const PERC_HARM: EnumParameter =
    EnumParameter::new(FourCC(*b"pHrm"), "Perc Harmonic", &["2nd", "3rd"], 1);

// -------------------------------------------------------------------------------------------------

pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMS: [&dyn Parameter; 18] = [
        &DRAWBAR_16,
        &DRAWBAR_8,
        &DRAWBAR_5_1_3,
        &DRAWBAR_4,
        &DRAWBAR_2_2_3,
        &DRAWBAR_2,
        &TUNE_16,
        &TUNE_5_1_3,
        &TUNE_4,
        &TUNE_2_2_3,
        &TUNE_2,
        &ATTACK,
        &RELEASE,
        &VIB_RATE,
        &VIB_DEPTH,
        &PERC_LEVEL,
        &PERC_DECAY,
        &PERC_HARM,
    ];
    &ALL_PARAMS
}

#[allow(unused)]
pub fn randomize() -> Vec<(FourCC, f32)> {
    let mut updates = Vec::new();
    for param in parameters() {
        let id = param.id();
        let value = if id == DRAWBAR_8.id() {
            1.0
        } else if id == DRAWBAR_16.id() || id == DRAWBAR_4.id() {
            rand::random_range(0.0..1.0)
        } else if id == DRAWBAR_5_1_3.id() || id == DRAWBAR_2_2_3.id() || id == DRAWBAR_2.id() {
            if rand::random_range(0.0..1.0) < 0.5 {
                0.0
            } else {
                rand::random_range(0.0..0.6)
            }
        } else if id == TUNE_16.id()
            || id == TUNE_5_1_3.id()
            || id == TUNE_4.id()
            || id == TUNE_2_2_3.id()
            || id == TUNE_2.id()
        {
            let val = rand::random_range(-5.0..5.0);
            TUNE_16.normalize_value(val)
        } else if id == ATTACK.id() {
            let val = rand::random_range(0.001..0.05);
            ATTACK.normalize_value(val)
        } else if id == RELEASE.id() {
            let val = rand::random_range(0.05..0.5);
            RELEASE.normalize_value(val)
        } else if id == VIB_DEPTH.id() {
            if rand::random_range(0.0..1.0) < 0.3 {
                rand::random_range(0.0..0.2)
            } else {
                0.0
            }
        } else if id == PERC_LEVEL.id() {
            if rand::random_range(0.0..1.0) < 0.3 {
                rand::random_range(0.3..0.8)
            } else {
                0.0
            }
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
    volume: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
) -> Box<dyn AudioUnit> {
    // Vibrato
    let vib_rate = var(&parameter(VIB_RATE.id()));
    let vib_depth = var(&parameter(VIB_DEPTH.id()));
    let vibrato = (vib_rate >> sine()) * vib_depth * 0.03; // approx +/- 3% pitch mod
    let freq_mod = var(&freq) * (1.0 + vibrato);

    // Helper to create harmonics
    let make_tuned_harmonic = |ratio: f32, tune_param: Shared, level_param: Shared| {
        let tune_mult = var(&tune_param) >> shape_fn(|cents| 2.0f32.powf(cents / 1200.0));
        let h_freq = freq_mod.clone() * ratio * tune_mult;
        let h_sig = h_freq >> sine();
        h_sig * var(&level_param)
    };
    let make_harmonic = |ratio: f32, level_param: Shared| {
        let h_freq = freq_mod.clone() * ratio;
        let h_sig = h_freq >> sine();
        h_sig * var(&level_param)
    };

    // Drawbars
    let d16 = make_tuned_harmonic(0.5, parameter(TUNE_16.id()), parameter(DRAWBAR_16.id()));
    let d8 = make_harmonic(1.0, parameter(DRAWBAR_8.id()));
    let d53 = make_tuned_harmonic(
        1.5,
        parameter(TUNE_5_1_3.id()),
        parameter(DRAWBAR_5_1_3.id()),
    );
    let d4 = make_tuned_harmonic(2.0, parameter(TUNE_4.id()), parameter(DRAWBAR_4.id()));
    let d23 = make_tuned_harmonic(
        3.0,
        parameter(TUNE_2_2_3.id()),
        parameter(DRAWBAR_2_2_3.id()),
    );
    let d2 = make_tuned_harmonic(4.0, parameter(TUNE_2.id()), parameter(DRAWBAR_2.id()));

    let organ_tone = (d16 + d8 + d53 + d4 + d23 + d2) * 0.3;

    // Percussion
    let perc_harm_sel = var(&parameter(PERC_HARM.id()));
    // 0 -> 2nd (4'), 1 -> 3rd (2 2/3')
    let perc_freq = freq_mod.clone()
        * (perc_harm_sel >> shape_fn(|x| if x.round() == 0.0 { 2.0 } else { 3.0 }));
    let perc_sig = perc_freq >> sine();

    let perc_env = shared_ahdsr(
        gate.clone(),
        shared(0.001), // Attack
        shared(0.0),   // Hold
        parameter(PERC_DECAY.id()),
        shared(0.0),  // Sustain
        shared(0.01), // Release
    );
    let perc_sound = perc_sig * perc_env * var(&parameter(PERC_LEVEL.id()));

    // Main Envelope
    let main_env = shared_ahdsr(
        gate,
        parameter(ATTACK.id()),
        shared(0.0), // Hold
        shared(0.0), // Decay
        shared(1.0), // Sustain
        parameter(RELEASE.id()),
    );

    // Mix
    let mixed = (organ_tone + perc_sound) * main_env;

    // Output
    let final_mix = ((mixed * var(&volume)) | var(&panning)) >> panner();
    Box::new(final_mix)
}
