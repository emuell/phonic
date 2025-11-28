//! Fundsp mono FM synth with controllable parameters. To be wrapped into a [`FunDspGenerator`].
//!
//! Provides:
//! - 3 oscillator frequencies (Carrier A, Modulator B, Modulator C).
//! - 3 modulation depths (B->A, C->A, C->B).
//! - 3 AHDSR envelopes (one per oscillator).
//!
//! A randomize function changes the sound via the exposed parameters.

use four_cc::FourCC;
use fundsp::hacker32::*;

use phonic::{
    generators::shared_ahdsr, parameters::FloatParameter, Error, GeneratorPlaybackHandle,
    Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

// Frequency parameters
pub const FREQ_A: FloatParameter = FloatParameter::new(
    FourCC(*b"frqA"),
    "Carrier A Frequency",
    20.0..=20000.0,
    440.0,
)
.with_unit("Hz")
.with_scaling(ParameterScaling::Exponential(2.0));
pub const FREQ_B: FloatParameter = FloatParameter::new(
    FourCC(*b"frqB"),
    "Modulator B Frequency",
    20.0..=20000.0,
    880.0,
)
.with_unit("Hz")
.with_scaling(ParameterScaling::Exponential(2.0));
pub const FREQ_C: FloatParameter = FloatParameter::new(
    FourCC(*b"frqC"),
    "Modulator C Frequency",
    20.0..=20000.0,
    10000.0,
)
.with_unit("Hz")
.with_scaling(ParameterScaling::Exponential(2.0));

// Modulation depth parameters
pub const DEPTH_B_TO_A: FloatParameter = FloatParameter::new(
    FourCC(*b"dpBA"), //
    "Modulation Depth B→A",
    0.0..=10.0,
    5.0,
);
pub const DEPTH_C_TO_A: FloatParameter = FloatParameter::new(
    FourCC(*b"dpCA"), //
    "Modulation Depth C→A",
    0.0..=10.0,
    3.0,
);
pub const DEPTH_C_TO_B: FloatParameter = FloatParameter::new(
    FourCC(*b"dpCB"), //
    "Modulation Depth C→B",
    0.0..=10.0,
    2.0,
);

// Operator A envelope parameters
pub const ATTACK_A: FloatParameter = FloatParameter::new(
    FourCC(*b"atA_"), //
    "Carrier A Attack",
    0.001..=5.0,
    0.01,
)
.with_unit("s");
pub const HOLD_A: FloatParameter = FloatParameter::new(
    FourCC(*b"hoA_"), //
    "Carrier A Hold",
    0.0..=5.0,
    0.0,
)
.with_unit("s");
pub const DECAY_A: FloatParameter = FloatParameter::new(
    FourCC(*b"dcA_"), //
    "Carrier A Decay",
    0.001..=5.0,
    0.1,
)
.with_unit("s");
pub const SUSTAIN_A: FloatParameter = FloatParameter::new(
    FourCC(*b"suA_"), //
    "Carrier A Sustain",
    0.0..=1.0,
    0.7,
);
pub const RELEASE_A: FloatParameter = FloatParameter::new(
    FourCC(*b"rlA_"), //
    "Carrier A Release",
    0.001..=5.0,
    0.5,
)
.with_unit("s");

// Operator B envelope parameters
pub const ATTACK_B: FloatParameter = FloatParameter::new(
    FourCC(*b"atB_"), //
    "Modulator B Attack",
    0.001..=5.0,
    0.01,
)
.with_unit("s");

pub const HOLD_B: FloatParameter = FloatParameter::new(
    FourCC(*b"hoB_"), //
    "Modulator B Hold",
    0.0..=5.0,
    0.0,
)
.with_unit("s");
pub const DECAY_B: FloatParameter = FloatParameter::new(
    FourCC(*b"dcB_"), //
    "Modulator B Decay",
    0.001..=5.0,
    0.1,
)
.with_unit("s");
pub const SUSTAIN_B: FloatParameter = FloatParameter::new(
    FourCC(*b"suB_"), //
    "Modulator B Sustain",
    0.0..=1.0,
    0.7,
);
pub const RELEASE_B: FloatParameter = FloatParameter::new(
    FourCC(*b"rlB_"), //
    "Modulator B Release",
    0.001..=5.0,
    0.5,
)
.with_unit("s");

// Operator C envelope parameters
pub const ATTACK_C: FloatParameter = FloatParameter::new(
    FourCC(*b"atC_"), //
    "Modulator C Attack",
    0.001..=5.0,
    0.01,
)
.with_unit("s");
pub const HOLD_C: FloatParameter = FloatParameter::new(
    FourCC(*b"hoC_"), //
    "Modulator C Hold",
    0.0..=5.0,
    0.0,
)
.with_unit("s");
pub const DECAY_C: FloatParameter = FloatParameter::new(
    FourCC(*b"dcC_"), //
    "Modulator C Decay",
    0.001..=5.0,
    0.1,
)
.with_unit("s");
pub const SUSTAIN_C: FloatParameter = FloatParameter::new(
    FourCC(*b"suC_"), //
    "Modulator C Sustain",
    0.0..=1.0,
    0.7,
);
pub const RELEASE_C: FloatParameter = FloatParameter::new(
    FourCC(*b"rlC_"), //
    "Modulator C Release",
    0.001..=5.0,
    0.5,
)
.with_unit("s");

// -------------------------------------------------------------------------------------------------

/// Exposes all automateable parameters.
pub fn parameters() -> Vec<FloatParameter> {
    vec![
        FREQ_A,
        FREQ_B,
        FREQ_C,
        DEPTH_B_TO_A,
        DEPTH_C_TO_A,
        DEPTH_C_TO_B,
        ATTACK_A,
        HOLD_A,
        DECAY_A,
        SUSTAIN_A,
        RELEASE_A,
        ATTACK_B,
        HOLD_B,
        DECAY_B,
        SUSTAIN_B,
        RELEASE_B,
        ATTACK_C,
        HOLD_C,
        DECAY_C,
        SUSTAIN_C,
        RELEASE_C,
    ]
}

// -------------------------------------------------------------------------------------------------

/// Randomize all parameters.
pub fn randomize(generator: &GeneratorPlaybackHandle) -> Result<(), Error> {
    for param in parameters().iter().filter(|p| p.id() != FREQ_A.id()) {
        generator.set_parameter(param.id(), rand::random_range(param.range().clone()), None)?;
    }
    Ok(())
}

// -------------------------------------------------------------------------------------------------

/// Create a new voice for a `FunDspGenerator`.
pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    vol: Shared,
    _pan: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
) -> Box<dyn AudioUnit> {
    // Get base frequencies from parameters (scaled by note frequency)
    let freq_a = var(&parameter(FREQ_A.id())) * var(&freq) * (1.0 / 440.0);
    let freq_b = var(&parameter(FREQ_B.id())) * var(&freq) * (1.0 / 440.0);
    let freq_c = var(&parameter(FREQ_C.id())) * var(&freq) * (1.0 / 440.0);

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

    // Operator C: simple sine wave with envelope
    let op_c = (freq_c >> sine()) * env_c;

    // Operator B: frequency-modulated by C with envelope
    // The modulation depth needs to be scaled to phase increments
    let op_b = ((freq_b.clone()
        + (op_c.clone() * var(&parameter(DEPTH_C_TO_B.id())) * freq_b.clone()))
        >> sine())
        * env_b;

    // Operator A: frequency-modulated by both B and C with envelope
    // Both modulation sources need to be scaled to phase increments
    let op_a = ((freq_a.clone()
        + (op_b * var(&parameter(DEPTH_B_TO_A.id())) * freq_a.clone())
        + (op_c * var(&parameter(DEPTH_C_TO_A.id())) * freq_a.clone()))
        >> sine())
        * env_a;

    // Final output with volume and skip panning control (keep signal mono)
    Box::new(op_a * var(&vol) * 0.3)
}
