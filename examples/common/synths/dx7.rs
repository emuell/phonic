//! DX7-inspired 6-Operator FM Synth.
//!
//! - 6 Operators with AHDSR envelopes.
//! - 32 Algorithms (routing configurations).
//! - Custom AudioNode for the FM matrix calculation.
//! - Feedback loop.

use phonic::{
    four_cc::FourCC,
    fundsp::{prelude32::*, shared::Shared},
    parameters::{EnumParameter, FloatParameter, IntegerParameter},
    Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

// GLOBAL

pub const ALGORITHM: EnumParameter = EnumParameter::new(
    FourCC(*b"algo"),
    "Algorithm",
    &[
        "1: 2→1, 6→5→4→3→1 (6↻)",
        "2: 2→1 (2↻), 6→5→4→3→1",
        "3: 6→5→4→1, 3→2→1 (6↻)",
        "4: 6→5→4→1, 3→2→1 (4↻)",
        "5: 6→5→4→1, 3→1, 2→1 (2↻)",
        "6: 6→5→4→1, 3→1, 2→1 (5↻)",
        "7: 6→5→4→3→1, 2→1 (6↻)",
        "8: 6→5→4→3→1, 2→1 (4↻)",
        "9: 6→5→1, 4→1, 3→1, 2→1 (6↻)",
        "10: 6→5→1, 4→1, 3→1 (3↻), 2→1",
        "11: 6→1, 5→1, 4→1, 3→1, 2→1 (6↻)",
        "12: 6→5→4→3, 2→1 (2↻)",
        "13: 6→5→4→3 (6↻), 2→1",
        "14: 6→5→4→3 (4↻), 2→1",
        "15: 6→5, 4→3, 2→1 (2↻)",
        "16: 6→5, 4→3 (4↻), 2→1",
        "17: 6→5 (6↻), 4→3, 2→1",
        "18: 6→5→4, 3→2→1 (3↻)",
        "19: 6→5→4 (6↻), 3→2→1",
        "20: 6 (6↻), 5→4→3, 2→1",
        "21: 6, 5→4→3 (5↻), 2→1",
        "22: 6, 5→4→3, 2→1 (2↻)",
        "23: 6→5 (6↻), 4→3, 2→1",
        "24: 6→5, 4→3 (4↻), 2→1",
        "25: 6→5, 4→3, 2→1 (2↻)",
        "26: 6 (6↻), 5, 4→3, 2→1",
        "27: 6→5 (6↻), 4, 3, 2→1",
        "28: 6→5 (6↻), 4, 3, 2, 1",
        "29: 6 (6↻), 5, 4, 3, 2, 1",
        "30: 6→5→4, 3→2→1 (3↻)",
        "31: 6→5 (6↻), 4→3→2→1",
        "32: 6→5→4→3→2→1 (6↻)",
    ],
    0,
);

pub const FEEDBACK: FloatParameter =
    FloatParameter::new(FourCC(*b"fdbk"), "Feedback", 0.0..=7.0, 0.0);

pub const LFO_RATE: FloatParameter =
    FloatParameter::new(FourCC(*b"lfoR"), "LFO Rate", 0.1..=20.0, 6.0)
        .with_unit("Hz")
        .with_scaling(ParameterScaling::Exponential(2.0));

pub const LFO_PITCH_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"lfoP"), "LFO Pitch Depth", 0.0..=12.0, 0.0).with_unit("st");

pub const LFO_AMP_DEPTH: FloatParameter =
    FloatParameter::new(FourCC(*b"lfoA"), "LFO Amp Depth", 0.0..=1.0, 0.0);

// OPERATOR 1

pub const OP1_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Lv"), "Op1 Level", 0.0..=1.0, 1.0);
pub const OP1_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o1Cr"), "Op1 Coarse", 0..=31, 1);
pub const OP1_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Fn"), "Op1 Fine", 0.0..=1.0, 0.0);
pub const OP1_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o1At"), "Op1 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP1_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Ho"), "Op1 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP1_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Dc"), "Op1 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP1_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Su"), "Op1 Sustain", 0.0..=1.0, 1.0);
pub const OP1_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o1Rl"), "Op1 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// OPERATOR 2

pub const OP2_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Lv"), "Op2 Level", 0.0..=1.0, 1.0);
pub const OP2_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o2Cr"), "Op2 Coarse", 0..=31, 1);
pub const OP2_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Fn"), "Op2 Fine", 0.0..=1.0, 0.0);
pub const OP2_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o2At"), "Op2 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP2_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Ho"), "Op2 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP2_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Dc"), "Op2 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP2_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Su"), "Op2 Sustain", 0.0..=1.0, 1.0);
pub const OP2_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o2Rl"), "Op2 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// OPERATOR 3

pub const OP3_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Lv"), "Op3 Level", 0.0..=1.0, 1.0);
pub const OP3_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o3Cr"), "Op3 Coarse", 0..=31, 1);
pub const OP3_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Fn"), "Op3 Fine", 0.0..=1.0, 0.0);
pub const OP3_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o3At"), "Op3 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP3_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Ho"), "Op3 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP3_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Dc"), "Op3 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP3_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Su"), "Op3 Sustain", 0.0..=1.0, 1.0);
pub const OP3_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o3Rl"), "Op3 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// OPERATOR 4

pub const OP4_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Lv"), "Op4 Level", 0.0..=1.0, 1.0);
pub const OP4_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o4Cr"), "Op4 Coarse", 0..=31, 1);
pub const OP4_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Fn"), "Op4 Fine", 0.0..=1.0, 0.0);
pub const OP4_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o4At"), "Op4 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP4_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Ho"), "Op4 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP4_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Dc"), "Op4 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP4_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Su"), "Op4 Sustain", 0.0..=1.0, 1.0);
pub const OP4_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o4Rl"), "Op4 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// OPERATOR 5

pub const OP5_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Lv"), "Op5 Level", 0.0..=1.0, 1.0);
pub const OP5_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o5Cr"), "Op5 Coarse", 0..=31, 1);
pub const OP5_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Fn"), "Op5 Fine", 0.0..=1.0, 0.0);
pub const OP5_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o5At"), "Op5 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP5_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Ho"), "Op5 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP5_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Dc"), "Op5 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP5_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Su"), "Op5 Sustain", 0.0..=1.0, 1.0);
pub const OP5_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o5Rl"), "Op5 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// OPERATOR 6

pub const OP6_LEVEL: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Lv"), "Op6 Level", 0.0..=1.0, 1.0);
pub const OP6_COARSE: IntegerParameter =
    IntegerParameter::new(FourCC(*b"o6Cr"), "Op6 Coarse", 0..=31, 1);
pub const OP6_FINE: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Fn"), "Op6 Fine", 0.0..=1.0, 0.0);
pub const OP6_ATTACK: FloatParameter =
    FloatParameter::new(FourCC(*b"o6At"), "Op6 Attack", 0.0..=10.0, 0.01)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP6_HOLD: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Ho"), "Op6 Hold", 0.0..=10.0, 0.0).with_unit("s");
pub const OP6_DECAY: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Dc"), "Op6 Decay", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));
pub const OP6_SUSTAIN: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Su"), "Op6 Sustain", 0.0..=1.0, 1.0);
pub const OP6_RELEASE: FloatParameter =
    FloatParameter::new(FourCC(*b"o6Rl"), "Op6 Release", 0.0..=10.0, 0.1)
        .with_unit("s")
        .with_scaling(ParameterScaling::Exponential(2.0));

// -------------------------------------------------------------------------------------------------

pub fn parameters() -> &'static [&'static dyn Parameter] {
    static PARAMS: [&dyn Parameter; 53] = [
        &ALGORITHM,
        &FEEDBACK,
        &LFO_RATE,
        &LFO_PITCH_DEPTH,
        &LFO_AMP_DEPTH,
        &OP1_LEVEL,
        &OP1_COARSE,
        &OP1_FINE,
        &OP1_ATTACK,
        &OP1_HOLD,
        &OP1_DECAY,
        &OP1_SUSTAIN,
        &OP1_RELEASE,
        &OP2_LEVEL,
        &OP2_COARSE,
        &OP2_FINE,
        &OP2_ATTACK,
        &OP2_HOLD,
        &OP2_DECAY,
        &OP2_SUSTAIN,
        &OP2_RELEASE,
        &OP3_LEVEL,
        &OP3_COARSE,
        &OP3_FINE,
        &OP3_ATTACK,
        &OP3_HOLD,
        &OP3_DECAY,
        &OP3_SUSTAIN,
        &OP3_RELEASE,
        &OP4_LEVEL,
        &OP4_COARSE,
        &OP4_FINE,
        &OP4_ATTACK,
        &OP4_HOLD,
        &OP4_DECAY,
        &OP4_SUSTAIN,
        &OP4_RELEASE,
        &OP5_LEVEL,
        &OP5_COARSE,
        &OP5_FINE,
        &OP5_ATTACK,
        &OP5_HOLD,
        &OP5_DECAY,
        &OP5_SUSTAIN,
        &OP5_RELEASE,
        &OP6_LEVEL,
        &OP6_COARSE,
        &OP6_FINE,
        &OP6_ATTACK,
        &OP6_HOLD,
        &OP6_DECAY,
        &OP6_SUSTAIN,
        &OP6_RELEASE,
    ];
    &PARAMS
}

// -------------------------------------------------------------------------------------------------

/// Randomize all parameters to create some easy "interesting" FM sounds.
#[allow(unused)]
pub fn randomize() -> Vec<(FourCC, f32)> {
    let mut updates = Vec::new();

    // Pick Algorithm
    // We favor algorithms that are easier to control randomly.
    // Algo 4 (5 in DX7): 6->5->4->1, 3->1, 2->1. (3 stacks on 1 carrier). Good for EP.
    // Algo 15 (16 in DX7): 6->5, 4->3, 2->1. (3 parallel stacks). Good for Bells/Organs.
    // Algo 0 (1 in DX7): 6->5->4->3->2->1. (Deep stack). Complex/Noise/Bass.
    // Algo 31 (32 in DX7): All carriers. Organ.
    let algo_choices = [0, 4, 15, 31];
    let algo_idx = algo_choices[rand::random_range(0..algo_choices.len())];
    updates.push((
        ALGORITHM.id(),
        ALGORITHM.normalize_value(ALGORITHM.values()[algo_idx]),
    ));

    // Set Global Params
    let feedback = if rand::random_range(0.0..1.0) < 0.5 {
        rand::random_range(0.0..7.0)
    } else {
        0.0
    };
    updates.push((FEEDBACK.id(), FEEDBACK.normalize_value(feedback)));

    // Set LFO
    updates.push((
        LFO_RATE.id(),
        LFO_RATE.normalize_value(rand::random_range(0.1..8.0)),
    ));
    updates.push((LFO_PITCH_DEPTH.id(), 0.0)); // Keep pitch stable usually
    updates.push((LFO_AMP_DEPTH.id(), 0.0));

    // Operators
    let algo = &Algorithm::ALGORITHMS[algo_idx];

    for (i, op_config) in algo.operators.iter().enumerate() {
        // i is 0..5, corresponding to Op1..Op6
        // Map to params
        let (l, c, f, a, h, d, s, r) = match i + 1 {
            1 => (
                &OP1_LEVEL,
                &OP1_COARSE,
                &OP1_FINE,
                &OP1_ATTACK,
                &OP1_HOLD,
                &OP1_DECAY,
                &OP1_SUSTAIN,
                &OP1_RELEASE,
            ),
            2 => (
                &OP2_LEVEL,
                &OP2_COARSE,
                &OP2_FINE,
                &OP2_ATTACK,
                &OP2_HOLD,
                &OP2_DECAY,
                &OP2_SUSTAIN,
                &OP2_RELEASE,
            ),
            3 => (
                &OP3_LEVEL,
                &OP3_COARSE,
                &OP3_FINE,
                &OP3_ATTACK,
                &OP3_HOLD,
                &OP3_DECAY,
                &OP3_SUSTAIN,
                &OP3_RELEASE,
            ),
            4 => (
                &OP4_LEVEL,
                &OP4_COARSE,
                &OP4_FINE,
                &OP4_ATTACK,
                &OP4_HOLD,
                &OP4_DECAY,
                &OP4_SUSTAIN,
                &OP4_RELEASE,
            ),
            5 => (
                &OP5_LEVEL,
                &OP5_COARSE,
                &OP5_FINE,
                &OP5_ATTACK,
                &OP5_HOLD,
                &OP5_DECAY,
                &OP5_SUSTAIN,
                &OP5_RELEASE,
            ),
            6 => (
                &OP6_LEVEL,
                &OP6_COARSE,
                &OP6_FINE,
                &OP6_ATTACK,
                &OP6_HOLD,
                &OP6_DECAY,
                &OP6_SUSTAIN,
                &OP6_RELEASE,
            ),
            _ => unreachable!(),
        };

        if op_config.is_carrier {
            // Carrier Logic
            updates.push((l.id(), 1.0)); // Max level

            // Ratio: usually 1.0 (coarse=1), sometimes 0.5 or 2.0
            let coarse = if rand::random_range(0.0..1.0) < 0.8 {
                1
            } else {
                2
            };
            updates.push((c.id(), c.normalize_value(coarse as i32)));
            updates.push((f.id(), 0.0)); // No detune usually

            // Envelope
            // Pad vs Pluck decision?
            let is_pad = rand::random_range(0.0..1.0) < 0.4;
            if is_pad {
                updates.push((a.id(), a.normalize_value(rand::random_range(0.1..1.0))));
                updates.push((d.id(), d.normalize_value(0.0))); // Unused if sustain is 1
                updates.push((s.id(), 1.0));
                updates.push((r.id(), r.normalize_value(rand::random_range(0.5..2.0))));
            } else {
                // Pluck/Percussive
                updates.push((a.id(), a.normalize_value(0.001)));
                updates.push((d.id(), d.normalize_value(rand::random_range(0.2..2.0))));
                updates.push((s.id(), 0.0)); // Decay to silence
                updates.push((r.id(), r.normalize_value(0.1))); // Quick release if key lifted
            }
        } else {
            // Modulator Logic
            // Level defines brightness/timbre intensity
            let level = rand::random_range(0.5..1.0);
            updates.push((l.id(), level));

            // Ratio: Harmonics
            let coarse = rand::random_range(1..8);
            updates.push((c.id(), c.normalize_value(coarse as i32)));

            // Detune for thickness
            let fine = if rand::random_range(0.0..1.0) < 0.3 {
                rand::random_range(0.0..0.1)
            } else {
                0.0
            };
            updates.push((f.id(), fine));

            // Envelope: usually follows carrier or is shorter (pluck)
            updates.push((a.id(), a.normalize_value(rand::random_range(0.001..0.1))));
            updates.push((d.id(), d.normalize_value(rand::random_range(0.1..1.0))));
            updates.push((s.id(), rand::random_range(0.0..0.5))); // Lower sustain
            updates.push((r.id(), r.normalize_value(rand::random_range(0.1..0.5))));
        }

        updates.push((h.id(), 0.0)); // Hold usually 0
    }

    updates
}

// -------------------------------------------------------------------------------------------------

pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    vol: Shared,
    panning: Shared,
    parameter: &mut dyn FnMut(FourCC) -> Shared,
) -> Box<dyn AudioUnit> {
    // Helper to grab op params by index
    let mut get_op = |i: usize| -> OpParams {
        // Map index to the specific parameter constants
        // Note: This is a bit verbose but explicit.
        let (l, c, f, a, h, d, s, r) = match i {
            0 => (
                OP1_LEVEL.id(),
                OP1_COARSE.id(),
                OP1_FINE.id(),
                OP1_ATTACK.id(),
                OP1_HOLD.id(),
                OP1_DECAY.id(),
                OP1_SUSTAIN.id(),
                OP1_RELEASE.id(),
            ),
            1 => (
                OP2_LEVEL.id(),
                OP2_COARSE.id(),
                OP2_FINE.id(),
                OP2_ATTACK.id(),
                OP2_HOLD.id(),
                OP2_DECAY.id(),
                OP2_SUSTAIN.id(),
                OP2_RELEASE.id(),
            ),
            2 => (
                OP3_LEVEL.id(),
                OP3_COARSE.id(),
                OP3_FINE.id(),
                OP3_ATTACK.id(),
                OP3_HOLD.id(),
                OP3_DECAY.id(),
                OP3_SUSTAIN.id(),
                OP3_RELEASE.id(),
            ),
            3 => (
                OP4_LEVEL.id(),
                OP4_COARSE.id(),
                OP4_FINE.id(),
                OP4_ATTACK.id(),
                OP4_HOLD.id(),
                OP4_DECAY.id(),
                OP4_SUSTAIN.id(),
                OP4_RELEASE.id(),
            ),
            4 => (
                OP5_LEVEL.id(),
                OP5_COARSE.id(),
                OP5_FINE.id(),
                OP5_ATTACK.id(),
                OP5_HOLD.id(),
                OP5_DECAY.id(),
                OP5_SUSTAIN.id(),
                OP5_RELEASE.id(),
            ),
            5 => (
                OP6_LEVEL.id(),
                OP6_COARSE.id(),
                OP6_FINE.id(),
                OP6_ATTACK.id(),
                OP6_HOLD.id(),
                OP6_DECAY.id(),
                OP6_SUSTAIN.id(),
                OP6_RELEASE.id(),
            ),
            _ => panic!("Invalid operator index: {}", i),
        };

        OpParams {
            level: parameter(l),
            coarse: parameter(c),
            fine: parameter(f),
            attack: parameter(a),
            hold: parameter(h),
            decay: parameter(d),
            sustain: parameter(s),
            release: parameter(r),
        }
    };

    // Collect parameters for all 6 operators
    let op_params = [
        get_op(0),
        get_op(1),
        get_op(2),
        get_op(3),
        get_op(4),
        get_op(5),
    ];

    let params = Dx7Params {
        algorithm: parameter(ALGORITHM.id()),
        feedback: parameter(FEEDBACK.id()),
        lfo_rate: parameter(LFO_RATE.id()),
        lfo_pitch_depth: parameter(LFO_PITCH_DEPTH.id()),
        lfo_amp_depth: parameter(LFO_AMP_DEPTH.id()),
        ops: op_params,
    };

    // Create the custom DX7 node
    // Inputs: Frequency (0), Gate (1)
    let dx7_node = An(Dx7Node::new(params));

    // Graph: (Freq | Gate) -> Dx7 -> (Vol | Pan) -> Panner
    let synth = (var(&freq) | var(&gate)) >> dx7_node;
    let out = ((synth * var(&vol)) | var(&panning)) >> panner();

    Box::new(out)
}

// -------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct OpParams {
    level: Shared,
    coarse: Shared,
    fine: Shared,
    attack: Shared,
    hold: Shared,
    decay: Shared,
    sustain: Shared,
    release: Shared,
}

#[derive(Clone)]
struct Dx7Params {
    algorithm: Shared,
    feedback: Shared,
    lfo_rate: Shared,
    lfo_pitch_depth: Shared,
    lfo_amp_depth: Shared,
    ops: [OpParams; 6],
}

#[derive(Clone, Copy, Debug)]
struct OperatorConfig {
    modulators: [Option<usize>; 5], // Indices of modulators (0-5)
    modulator_count: usize,
    is_carrier: bool,
    feedback_in: bool,
}

// -------------------------------------------------------------------------------------------------

struct Algorithm {
    operators: [OperatorConfig; 6],
}

impl Algorithm {
    // Helper to define ops
    const fn op(
        modulators: [Option<usize>; 5],
        modulator_count: usize,
        is_carrier: bool,
        feedback_in: bool,
    ) -> OperatorConfig {
        OperatorConfig {
            modulators,
            modulator_count,
            is_carrier,
            feedback_in,
        }
    }

    // Note: DX7 algorithms usually number ops 6..1. Here we use indices 0..5 (Op1..Op6).
    // Standard DX7 Algo: `6->5->4->3->2->1`.
    // Here: `Op6(5)->Op5(4)->Op4(3)->Op3(2)->Op2(1)->Op1(0)`.
    // So Op0 is carrier.
    const ALGORITHMS: [Algorithm; 32] = [
        // 1: 2→1, 6→5→4→3→1 (6↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), None, None, None], 2, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 2: 2→1 (2↻), 6→5→4→3→1
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), None, None, None], 2, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 3: 6→5→4→1, 3→2→1 (6↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(3), None, None, None], 2, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 4: 6→5→4→1, 3→2→1 (4↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(3), None, None, None], 2, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, true),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 5: 6→5→4→1, 3→1, 2→1 (2↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), Some(3), None, None], 3, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 6: 6→5→4→1, 3→1, 2→1 (5↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), Some(3), None, None], 3, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, true),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 7: 6→5→4→3→1, 2→1 (6↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), None, None, None], 2, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 8: 6→5→4→3→1, 2→1 (4↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), None, None, None], 2, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, true),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 9: 6→5→1, 4→1, 3→1, 2→1 (6↻)
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), Some(3), Some(4), None], 4, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 10: 6→5→1, 4→1, 3→1 (3↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), Some(2), Some(3), Some(4), None], 4, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 11: 6→1, 5→1, 4→1, 3→1, 2→1 (6↻)
        Algorithm {
            operators: [
                Self::op(
                    [Some(1), Some(2), Some(3), Some(4), Some(5)],
                    5,
                    true,
                    false,
                ),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 12: 6→5→4→3, 2→1 (2↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 13: 6→5→4→3 (6↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 14: 6→5→4→3 (4↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, true),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 15: 6→5, 4→3, 2→1 (2↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 16: 6→5, 4→3 (4↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 17: 6→5 (6↻), 4→3, 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 18: 6→5→4, 3→2→1 (3↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(4), None, None, None, None], 1, true, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 19: 6→5→4 (6↻), 3→2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(4), None, None, None, None], 1, true, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 20: 6 (6↻), 5→4→3, 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, true, true),
            ],
        },
        // 21: 6, 5→4→3 (5↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([None, None, None, None, None], 0, true, false),
            ],
        },
        // 22: 6, 5→4→3, 2→1 (2↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, true, false),
            ],
        },
        // 23: 6→5 (6↻), 4→3, 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 24: 6→5, 4→3 (4↻), 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 25: 6→5, 4→3, 2→1 (2↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 26: 6 (6↻), 5, 4→3, 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(3), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, true),
            ],
        },
        // 27: 6→5 (6↻), 4, 3, 2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 28: 6→5 (6↻), 4, 3, 2, 1
        Algorithm {
            operators: [
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 29: 6 (6↻), 5, 4, 3, 2, 1
        Algorithm {
            operators: [
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, false),
                Self::op([None, None, None, None, None], 0, true, true),
            ],
        },
        // 30: 6→5→4, 3→2→1 (3↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
                Self::op([Some(4), None, None, None, None], 1, true, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
            ],
        },
        // 31: 6→5 (6↻), 4→3→2→1
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, false),
                Self::op([Some(5), None, None, None, None], 1, true, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
        // 32: 6→5→4→3→2→1 (6↻)
        Algorithm {
            operators: [
                Self::op([Some(1), None, None, None, None], 1, true, false),
                Self::op([Some(2), None, None, None, None], 1, false, false),
                Self::op([Some(3), None, None, None, None], 1, false, false),
                Self::op([Some(4), None, None, None, None], 1, false, false),
                Self::op([Some(5), None, None, None, None], 1, false, false),
                Self::op([None, None, None, None, None], 0, false, true),
            ],
        },
    ];
}

#[derive(Clone)]
struct OperatorState {
    phase: f32,
    env_stage: u8, // 0: Attack, 1: Hold, 2: Decay, 3: Sustain, 4: Release, 5: Idle
    env_level: f32,
    env_timer: f32,
    last_gate: f32,
}

impl OperatorState {
    fn new() -> Self {
        Self {
            phase: 0.0,
            env_stage: 5,
            env_level: 0.0,
            env_timer: 0.0,
            last_gate: 0.0,
        }
    }
}

#[derive(Clone)]
struct Dx7Node {
    params: Dx7Params,
    ops: [OperatorState; 6],
    sample_rate: f32,
    sample_duration: f32,
    // Feedback state
    feedback_mem: [f32; 2],
    // LFO state
    lfo_phase: f32,
}

impl Dx7Node {
    fn new(params: Dx7Params) -> Self {
        Self {
            params,
            ops: [
                OperatorState::new(),
                OperatorState::new(),
                OperatorState::new(),
                OperatorState::new(),
                OperatorState::new(),
                OperatorState::new(),
            ],
            sample_rate: 44100.0,
            sample_duration: 1.0 / 44100.0,
            feedback_mem: [0.0; 2],
            lfo_phase: 0.0,
        }
    }

    fn update_envelope(&mut self, op_idx: usize, gate: f32) -> f32 {
        let params = &self.params.ops[op_idx];
        let op = &mut self.ops[op_idx];

        // Gate trigger
        if gate > 0.5 && op.last_gate <= 0.5 {
            op.env_stage = 0;
            op.env_timer = 0.0;
        } else if gate <= 0.5 && op.last_gate > 0.5 {
            op.env_stage = 4;
            op.env_timer = 0.0;
        }
        op.last_gate = gate;

        let attack_time = params.attack.value();
        let hold_time = params.hold.value();
        let decay_time = params.decay.value();
        let sustain_level = params.sustain.value();
        let release_time = params.release.value();

        match op.env_stage {
            0 => {
                // Attack
                if attack_time <= 0.001 {
                    op.env_level = 1.0;
                    op.env_stage = 1;
                    op.env_timer = 0.0;
                } else {
                    op.env_level += self.sample_duration / attack_time;
                    if op.env_level >= 1.0 {
                        op.env_level = 1.0;
                        op.env_stage = 1;
                        op.env_timer = 0.0;
                    }
                }
            }
            1 => {
                // Hold
                op.env_timer += self.sample_duration;
                if op.env_timer >= hold_time {
                    op.env_stage = 2;
                    op.env_timer = 0.0;
                }
            }
            2 => {
                // Decay
                if decay_time <= 0.001 {
                    op.env_level = sustain_level;
                    op.env_stage = 3;
                } else {
                    let drop = (1.0 - sustain_level) * (self.sample_duration / decay_time);
                    op.env_level -= drop;
                    if op.env_level <= sustain_level {
                        op.env_level = sustain_level;
                        op.env_stage = 3;
                    }
                }
            }
            3 => {
                // Sustain
                op.env_level = sustain_level;
            }
            4 => {
                // Release
                if release_time <= 0.001 {
                    op.env_level = 0.0;
                    op.env_stage = 5;
                } else {
                    op.env_level -= self.sample_duration / release_time;
                    if op.env_level <= 0.0 {
                        op.env_level = 0.0;
                        op.env_stage = 5;
                    }
                }
            }
            _ => {
                op.env_level = 0.0;
            }
        }

        op.env_level
    }
}

impl AudioNode for Dx7Node {
    const ID: u64 = 0x445837; // "DX7"
    type Inputs = U2; // Freq, Gate
    type Outputs = U1; // Audio

    fn reset(&mut self) {
        for op in &mut self.ops {
            op.phase = 0.0;
            op.env_stage = 5;
            op.env_level = 0.0;
            op.last_gate = 0.0;
        }
        self.feedback_mem = [0.0; 2];
        self.lfo_phase = 0.0;
    }

    fn set_sample_rate(&mut self, sample_rate: f64) {
        self.sample_rate = sample_rate as f32;
        self.sample_duration = 1.0 / self.sample_rate;
    }

    #[inline]
    fn tick(&mut self, input: &Frame<f32, Self::Inputs>) -> Frame<f32, Self::Outputs> {
        let freq = input[0];
        let gate = input[1];

        let algo_idx = self.params.algorithm.value().round() as usize;
        let algo_idx = algo_idx.clamp(0, 31);
        let algo = &Algorithm::ALGORITHMS[algo_idx];
        let feedback_amt = self.params.feedback.value();

        // LFO Calculation
        let lfo_rate = self.params.lfo_rate.value();
        let lfo_pitch_depth = self.params.lfo_pitch_depth.value();
        let lfo_amp_depth = self.params.lfo_amp_depth.value();

        self.lfo_phase += lfo_rate * self.sample_duration;
        self.lfo_phase -= self.lfo_phase.floor();
        let lfo_val = (self.lfo_phase * std::f32::consts::TAU).sin(); // -1.0 to 1.0

        // Pitch Mod: 2^(semitones/12)
        let pitch_mod = if lfo_pitch_depth > 0.0 {
            2.0f32.powf(lfo_val * lfo_pitch_depth / 12.0)
        } else {
            1.0
        };

        // Amp Mod: Scale output. 0..1 depth.
        // Simple tremolo: 1.0 + lfo * depth * 0.5? Or 1.0 - depth * (1 - lfo)/2?
        // Let's use: 1.0 + lfo_val * lfo_amp_depth * 0.5 (varies around 1.0)
        // Or strictly attenuation?
        // Let's assume it modulates the carriers.
        let amp_mod = 1.0 + lfo_val * lfo_amp_depth * 0.5;

        // Calculate envelopes and frequencies first
        let mut op_envs = [0.0; 6];
        let mut op_freqs = [0.0; 6];

        for i in 0..6 {
            op_envs[i] = self.update_envelope(i, gate) * self.params.ops[i].level.value();

            let coarse = self.params.ops[i].coarse.value();
            let fine = self.params.ops[i].fine.value();
            // DX7 Ratio logic: Coarse 0=0.5, 1=1, 2=2...
            // Fine adds to ratio.
            let ratio = if coarse == 0.0 { 0.5 } else { coarse } + fine;
            op_freqs[i] = freq * ratio * pitch_mod;
        }

        // FM Matrix Calculation
        let mut op_outputs = [0.0; 6];

        // Feedback signal (average of last two samples)
        // Scaled by feedback amount.
        // DX7 feedback scaling is non-linear, but we use linear approx here.
        // 0..7 -> 0..something.
        let fb_in = (self.feedback_mem[0] + self.feedback_mem[1]) * 0.5 * (feedback_amt * 0.5);

        // We iterate backwards (Op6 to Op1)
        for i in (0..6).rev() {
            let config = &algo.operators[i];
            let mut mod_sum = 0.0;

            for m in 0..config.modulator_count {
                if let Some(mod_idx) = config.modulators[m] {
                    mod_sum += op_outputs[mod_idx];
                }
            }

            // Apply Feedback if this operator receives it
            if config.feedback_in {
                mod_sum += fb_in;
            }

            // Phase update
            let delta = op_freqs[i] * self.sample_duration;
            self.ops[i].phase += delta;
            self.ops[i].phase -= self.ops[i].phase.floor();

            // PM
            let phase_mod = mod_sum * 0.5; // Arbitrary scaling
            let phase_total = (self.ops[i].phase + phase_mod) * std::f32::consts::TAU;

            let output = phase_total.sin() * op_envs[i];
            op_outputs[i] = output;

            // If this operator is the feedback source (receives feedback), update memory
            if config.feedback_in {
                self.feedback_mem[1] = self.feedback_mem[0];
                self.feedback_mem[0] = output;
            }
        }

        // Sum carriers
        let mut output = 0.0;
        for (operator, op_output) in algo.operators.iter().zip(op_outputs.iter()) {
            if operator.is_carrier {
                output += op_output;
            }
        }

        // Apply global Amp Mod to output (or carriers)
        output *= amp_mod * 0.5;

        [output].into()
    }
}
