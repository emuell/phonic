//! Simple Fundsp organ synth with 6 drawbars, percussion, click and vibrato.
//! To be wrapped into a [`FunDspGenerator`].

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
pub const LFO_RATE: FloatParameter =
    FloatParameter::new(FourCC(*b"vRat"), "Vibrato Rate", 0.01..=20.0, 6.0).with_unit("Hz");
pub const LFO_WAVEFORM: EnumParameter = EnumParameter::new(
    FourCC(*b"vWav"),
    "Vibrato Waveform",
    LfoWaveform::VARIANTS,
    LfoWaveform::Sine as usize,
);

// Percussion
pub const PERC_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"pLvl"), "Perc Level", 0.0..=1.0, 0.0);
pub const PERC_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"pDec"), "Perc Decay", 0.01..=1.0, 0.2).with_unit("s");
pub const PERC_HARM: EnumParameter =
    EnumParameter::new(FourCC(*b"pHrm"), "Perc Harmonic", &["2nd", "3rd"], 1);

// -------------------------------------------------------------------------------------------------

// Modulation Source IDs
pub const MOD_SRC_VIBRATO: FourCC = FourCC(*b"VLFO");
pub const MOD_SRC_VELOCITY: FourCC = FourCC(*b"MVEL");
pub const MOD_SRC_KEYTRACK: FourCC = FourCC(*b"MKEY");

// Modulation Target IDs (virtual parameters for modulation)
pub const MOD_TARGET_PITCH: FourCC = FourCC(*b"mPit");
pub const MOD_TARGET_VOLUME: FourCC = FourCC(*b"mVol");
pub const MOD_TARGET_PERC_LEVEL: FourCC = FourCC(*b"mPLv");

// -------------------------------------------------------------------------------------------------

/// Exposes all automateable parameters. *excluding* modulation source parameters.
pub fn parameters() -> &'static [&'static dyn Parameter] {
    const ALL_PARAMS: [&dyn Parameter; 16] = [
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
        &PERC_LEVEL,
        &PERC_DECAY,
        &PERC_HARM,
    ];
    &ALL_PARAMS
}

// -------------------------------------------------------------------------------------------------

/// Returns the modulation configuration for the organ synth.
pub fn modulation_config() -> ModulationConfig {
    ModulationConfig {
        sources: vec![
            // LFO (Vibrato/Tremolo)
            ModulationSource::Lfo {
                id: MOD_SRC_VIBRATO,
                name: "LFO",
                rate_param: LFO_RATE,
                waveform_param: LFO_WAVEFORM,
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
            ModulationTarget::new(MOD_TARGET_PITCH, "Pitch"),
            ModulationTarget::new(MOD_TARGET_VOLUME, "Volume"),
            ModulationTarget::new(MOD_TARGET_PERC_LEVEL, "Perc Level"),
        ],
    }
}

// -------------------------------------------------------------------------------------------------

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

    for param in modulation_config().source_parameters() {
        let id = param.id();
        let value = if id == LFO_RATE.id() {
            let val = rand::random_range(4.0..8.0);
            LFO_RATE.normalize_value(val)
        } else if id == LFO_WAVEFORM.id() {
            // Favor Sine for vibrato, but allow Triangle
            if rand::random_range(0.0..1.0) < 0.8 {
                0.0 // Sine
            } else {
                1.0 / (LfoWaveform::VARIANTS.len() - 1) as f32 // Triangle
            }
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

    // Vibrato LFO -> Pitch: 30% chance of having vibrato
    if rand::random_range(0.0..1.0) < 0.3 {
        routes.push((
            MOD_SRC_VIBRATO,
            MOD_TARGET_PITCH,
            rand::random_range(0.2..0.6), // Subtle vibrato
            true,                         // bipolar
        ));
    }

    // Tremolo LFO -> Volume: 20% chance of tremolo
    if rand::random_range(0.0..1.0) < 0.2 {
        routes.push((
            MOD_SRC_VIBRATO,
            MOD_TARGET_VOLUME,
            rand::random_range(0.3..0.7), // Moderate tremolo
            true,                         // bipolar
        ));
    }

    // Velocity -> Perc Level: 40% chance
    if rand::random_range(0.0..1.0) < 0.4 {
        routes.push((
            MOD_SRC_VELOCITY,
            MOD_TARGET_PERC_LEVEL,
            rand::random_range(0.4..0.8), // Strong velocity sensitivity
            false,                        // unipolar
        ));
    }

    // Keytracking -> Volume: 30% chance (brighter notes louder)
    if rand::random_range(0.0..1.0) < 0.3 {
        routes.push((
            MOD_SRC_KEYTRACK,
            MOD_TARGET_VOLUME,
            rand::random_range(0.2..0.5), // Subtle keytracking
            false,                        // unipolar
        ));
    }

    routes
}

// -------------------------------------------------------------------------------------------------

pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    volume: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
    modulation: &mut dyn FnMut(FourCC) -> SharedBuffer,
) -> Box<dyn AudioUnit> {
    // Get modulation buffers
    let pitch_mod_buffer = modulation(MOD_TARGET_PITCH);
    let volume_mod_buffer = modulation(MOD_TARGET_VOLUME);
    let perc_level_mod_buffer = modulation(MOD_TARGET_PERC_LEVEL);

    // Vibrato modulation (±3% pitch mod, approx ±0.5 semitone)
    let vibrato = var_buffer(&pitch_mod_buffer) * 0.03;
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
    // Apply percussion level modulation (unipolar 0..1)
    let perc_level_mod = var_buffer(&perc_level_mod_buffer);
    let perc_level = var(&parameter(PERC_LEVEL.id())) * (1.0 + perc_level_mod);
    let perc_sound = perc_sig * perc_env * perc_level;

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

    // Apply volume modulation (tremolo) - unipolar 0..1
    let volume_mod = var_buffer(&volume_mod_buffer);
    let modulated_volume = var(&volume) * (1.0 + volume_mod);

    // Output
    let final_mix = ((mixed * modulated_volume) | var(&panning)) >> panner();
    Box::new(final_mix)
}
