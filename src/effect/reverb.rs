use assume::assume;
use four_cc::FourCC;
use rand::Rng;
use std::any::Any;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{FloatParameter, ParameterValueUpdate, SmoothedParameterValue},
    utils::{
        buffer::InterleavedBufferMut,
        dsp::{
            delay::{AllpassDelayLine, DelayLine},
            filters::biquad::{BiquadFilter, BiquadFilterCoefficients, BiquadFilterType},
        },
        smoothing::{ExponentialSmoothedValue, LinearSmoothedValue},
    },
    Error, Parameter,
};

// -------------------------------------------------------------------------------------------------

/// Message type for `ReverbEffect` to change parameters.
#[derive(Clone, Debug)]
pub enum ReverbEffectMessage {
    /// Reset/clear all delay lines
    Reset,
}

impl EffectMessage for ReverbEffectMessage {
    fn effect_name(&self) -> &'static str {
        ReverbEffect::EFFECT_NAME
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// Stereo reverb effect ported from [Airwindows' Reverb plugin](https://www.airwindows.com/reverb/).
pub struct ReverbEffect {
    sample_rate: u32,
    channel_count: usize,
    room_size: SmoothedParameterValue<LinearSmoothedValue>,
    wet: SmoothedParameterValue<ExponentialSmoothedValue>,

    biquad_a_coefficients: BiquadFilterCoefficients,
    biquad_a_l: BiquadFilter,
    biquad_a_r: BiquadFilter,
    biquad_b_coefficients: BiquadFilterCoefficients,
    biquad_b_l: BiquadFilter,
    biquad_b_r: BiquadFilter,
    biquad_c_coefficients: BiquadFilterCoefficients,
    biquad_c_l: BiquadFilter,
    biquad_c_r: BiquadFilter,

    fpd_l: u32,
    fpd_r: u32,

    a: Box<ReverbDelayLine<2>>,
    b: Box<ReverbDelayLine<2>>,
    c: Box<ReverbDelayLine<2>>,
    d: Box<ReverbDelayLine<2>>,
    e: Box<ReverbDelayLine<2>>,
    f: Box<ReverbDelayLine<2>>,
    g: Box<ReverbDelayLine<2>>,
    h: Box<ReverbDelayLine<2>>,
    i: Box<AllpassDelayLine<2>>,
    j: Box<AllpassDelayLine<2>>,
    k: Box<AllpassDelayLine<2>>,
    l: Box<AllpassDelayLine<2>>,
    m: Box<DelayLine<2>>,
}

impl ReverbEffect {
    pub const EFFECT_NAME: &str = "Reverb";

    pub const ROOM_SIZE: FloatParameter = FloatParameter::new(
        FourCC(*b"room"),
        "Room Size",
        0.0..=1.0,
        0.6, //
    )
    .with_unit("%");
    pub const WET: FloatParameter = FloatParameter::new(
        FourCC(*b"wet "),
        "Wet",
        0.0..=1.0,
        0.35, //
    )
    .with_unit("%");

    /// Creates a new `ReverbEffect` with default parameter values.
    pub fn new() -> Self {
        let mut rng = rand::rng();
        let mut fpd_l = 1;
        while fpd_l < 16386 {
            fpd_l = rng.random();
        }
        let mut fpd_r = 1;
        while fpd_r < 16386 {
            fpd_r = rng.random();
        }

        let to_string_percent = |v: f32| format!("{:.2}", v * 100.0);
        let from_string_percent = |v: &str| v.parse::<f32>().map(|f| f / 100.0).ok();

        /// Max Delay line sizes
        const A_SIZE: usize = 8111;
        const B_SIZE: usize = 7511;
        const C_SIZE: usize = 7311;
        const D_SIZE: usize = 6911;
        const E_SIZE: usize = 6311;
        const F_SIZE: usize = 6111;
        const G_SIZE: usize = 5511;
        const H_SIZE: usize = 4911;
        const I_SIZE: usize = 4511;
        const J_SIZE: usize = 4311;
        const K_SIZE: usize = 3911;
        const L_SIZE: usize = 3311;
        const M_SIZE: usize = 3111;

        Self {
            sample_rate: 0,
            channel_count: 0,
            room_size: SmoothedParameterValue::from_description(
                Self::ROOM_SIZE
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            )
            .with_smoother(LinearSmoothedValue::default().with_step(0.01)),
            wet: SmoothedParameterValue::from_description(
                Self::WET
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            biquad_a_coefficients: BiquadFilterCoefficients::default(),
            biquad_a_l: BiquadFilter::default(),
            biquad_a_r: BiquadFilter::default(),
            biquad_b_coefficients: BiquadFilterCoefficients::default(),
            biquad_b_l: BiquadFilter::default(),
            biquad_b_r: BiquadFilter::default(),
            biquad_c_coefficients: BiquadFilterCoefficients::default(),
            biquad_c_l: BiquadFilter::default(),
            biquad_c_r: BiquadFilter::default(),
            fpd_l,
            fpd_r,
            a: Box::new(ReverbDelayLine::new(&mut rng, A_SIZE, 0.003251)),
            b: Box::new(ReverbDelayLine::new(&mut rng, B_SIZE, 0.002999)),
            c: Box::new(ReverbDelayLine::new(&mut rng, C_SIZE, 0.002917)),
            d: Box::new(ReverbDelayLine::new(&mut rng, D_SIZE, 0.002749)),
            e: Box::new(ReverbDelayLine::new(&mut rng, E_SIZE, 0.002503)),
            f: Box::new(ReverbDelayLine::new(&mut rng, F_SIZE, 0.002423)),
            g: Box::new(ReverbDelayLine::new(&mut rng, G_SIZE, 0.002146)),
            h: Box::new(ReverbDelayLine::new(&mut rng, H_SIZE, 0.002088)),
            i: Box::new(AllpassDelayLine::new(I_SIZE)),
            j: Box::new(AllpassDelayLine::new(J_SIZE)),
            k: Box::new(AllpassDelayLine::new(K_SIZE)),
            l: Box::new(AllpassDelayLine::new(L_SIZE)),
            m: Box::new(DelayLine::new(M_SIZE)),
        }
    }

    /// Creates a new `ReverbEffect` with the given parameters.
    pub fn with_parameters(room_size: f32, wet: f32) -> Self {
        let mut reverb = Self::new();
        reverb.room_size.init_value(room_size);
        reverb.wet.init_value(wet);
        reverb
    }

    fn update_filter_coefs(&mut self, cutoff: f32) {
        if let Err(err) = self
            .biquad_a_coefficients
            .set(
                BiquadFilterType::Lowpass,
                self.sample_rate,
                cutoff,
                1.618034,
                0.0,
            )
            .and_then(|_| {
                self.biquad_b_coefficients.set(
                    BiquadFilterType::Lowpass,
                    self.sample_rate,
                    cutoff,
                    0.618034,
                    0.0,
                )
            })
            .and_then(|_| {
                self.biquad_c_coefficients.set(
                    //
                    BiquadFilterType::Lowpass,
                    self.sample_rate,
                    cutoff,
                    0.5,
                    0.0,
                )
            })
        {
            log::error!("Failed to set biquad coefs in reverb effect: {err}");
        };
    }

    fn update_delay_sizes(&mut self, size: f64) -> usize {
        // reverb delays
        self.a.set_delay((79.0 * size) as usize);
        self.b.set_delay((73.0 * size) as usize);
        self.c.set_delay((71.0 * size) as usize);
        self.d.set_delay((67.0 * size) as usize);
        self.e.set_delay((61.0 * size) as usize);
        self.f.set_delay((59.0 * size) as usize);
        self.g.set_delay((53.0 * size) as usize);
        self.h.set_delay((47.0 * size) as usize);
        // allpass
        self.i.set_delay((43.0 * size) as usize);
        self.j.set_delay((41.0 * size) as usize);
        self.k.set_delay((37.0 * size) as usize);
        self.l.set_delay((31.0 * size) as usize);
        // predelay
        (29.0 * size) as usize
    }

    /// Process a single stereo sample frame with the given parameters
    #[inline(always)]
    fn process_frame(
        &mut self,
        frame: &mut [f32; 2],
        blend: f64,
        regen: f64,
        predelay: usize,
        wet: f64,
    ) {
        let vib_speed = 0.1;
        let vib_depth = 7.0;

        let mut input_l = frame[0] as f64;
        let mut input_r = frame[1] as f64;

        if input_l.abs() < 1.18e-23 {
            input_l = self.fpd_l as f64 * 1.18e-17;
        }
        if input_r.abs() < 1.18e-23 {
            input_r = self.fpd_r as f64 * 1.18e-17;
        }
        let dry_l = input_l;
        let dry_r = input_r;

        // Predelay
        let predelay_out = self.m.process(predelay, [input_l, input_r]);
        input_l = predelay_out[0];
        input_r = predelay_out[1];

        // Biquad A
        input_l = self
            .biquad_a_l
            .process_sample(&self.biquad_a_coefficients, input_l);
        input_r = self
            .biquad_a_r
            .process_sample(&self.biquad_a_coefficients, input_r);

        input_l *= wet;
        input_r *= wet;

        input_l = input_l.sin();
        input_r = input_r.sin();

        // Allpasses
        let out_i = self.i.process([input_l, input_r]);
        let out_j = self.j.process(out_i);
        let out_k = self.k.process(out_j);
        let out_l = self.l.process(out_k);

        let allpass_il = out_i[0];
        let allpass_ir = out_i[1];
        let allpass_jl = out_j[0];
        let allpass_jr = out_j[1];
        let allpass_kl = out_k[0];
        let allpass_kr = out_k[1];
        let allpass_ll = out_l[0];
        let allpass_lr = out_l[1];

        // Householder Matrix
        self.a.set([allpass_ll, allpass_lr]);
        self.b.set([allpass_kl, allpass_kr]);
        self.c.set([allpass_jl, allpass_jr]);
        self.d.set([allpass_il, allpass_ir]);
        self.e.set([allpass_il, allpass_ir]);
        self.f.set([allpass_jl, allpass_jr]);
        self.g.set([allpass_kl, allpass_kr]);
        self.h.set([allpass_ll, allpass_lr]);

        self.a.step(vib_speed);
        self.b.step(vib_speed);
        self.c.step(vib_speed);
        self.d.step(vib_speed);
        self.e.step(vib_speed);
        self.f.step(vib_speed);
        self.g.step(vib_speed);
        self.h.step(vib_speed);

        let [interpol_al, interpol_ar] = self.a.get(vib_depth, blend);
        let [interpol_bl, interpol_br] = self.b.get(vib_depth, blend);
        let [interpol_cl, interpol_cr] = self.c.get(vib_depth, blend);
        let [interpol_dl, interpol_dr] = self.d.get(vib_depth, blend);
        let [interpol_el, interpol_er] = self.e.get(vib_depth, blend);
        let [interpol_fl, interpol_fr] = self.f.get(vib_depth, blend);
        let [interpol_gl, interpol_gr] = self.g.get(vib_depth, blend);
        let [interpol_hl, interpol_hr] = self.h.get(vib_depth, blend);

        // Feedback
        self.a.feedback[0] = (interpol_al - (interpol_bl + interpol_cl + interpol_dl)) * regen;
        self.b.feedback[0] = (interpol_bl - (interpol_al + interpol_cl + interpol_dl)) * regen;
        self.c.feedback[0] = (interpol_cl - (interpol_al + interpol_bl + interpol_dl)) * regen;
        self.d.feedback[0] = (interpol_dl - (interpol_al + interpol_bl + interpol_cl)) * regen;
        self.e.feedback[0] = (interpol_el - (interpol_fl + interpol_gl + interpol_hl)) * regen;
        self.f.feedback[0] = (interpol_fl - (interpol_el + interpol_gl + interpol_hl)) * regen;
        self.g.feedback[0] = (interpol_gl - (interpol_el + interpol_fl + interpol_hl)) * regen;
        self.h.feedback[0] = (interpol_hl - (interpol_el + interpol_fl + interpol_gl)) * regen;

        self.a.feedback[1] = (interpol_ar - (interpol_br + interpol_cr + interpol_dr)) * regen;
        self.b.feedback[1] = (interpol_br - (interpol_ar + interpol_cr + interpol_dr)) * regen;
        self.c.feedback[1] = (interpol_cr - (interpol_ar + interpol_br + interpol_dr)) * regen;
        self.d.feedback[1] = (interpol_dr - (interpol_ar + interpol_br + interpol_cr)) * regen;
        self.e.feedback[1] = (interpol_er - (interpol_fr + interpol_gr + interpol_hr)) * regen;
        self.f.feedback[1] = (interpol_fr - (interpol_er + interpol_gr + interpol_hr)) * regen;
        self.g.feedback[1] = (interpol_gr - (interpol_er + interpol_fr + interpol_hr)) * regen;
        self.h.feedback[1] = (interpol_hr - (interpol_er + interpol_fr + interpol_gr)) * regen;

        input_l = (interpol_al
            + interpol_bl
            + interpol_cl
            + interpol_dl
            + interpol_el
            + interpol_fl
            + interpol_gl
            + interpol_hl)
            / 8.0;
        input_r = (interpol_ar
            + interpol_br
            + interpol_cr
            + interpol_dr
            + interpol_er
            + interpol_fr
            + interpol_gr
            + interpol_hr)
            / 8.0;

        // Biquad B
        input_l = self
            .biquad_b_l
            .process_sample(&self.biquad_b_coefficients, input_l);
        input_r = self
            .biquad_b_r
            .process_sample(&self.biquad_b_coefficients, input_r);

        input_l = input_l.clamp(-1.0, 1.0);
        input_r = input_r.clamp(-1.0, 1.0);

        input_l = input_l.asin();
        input_r = input_r.asin();

        // Biquad C
        input_l = self
            .biquad_c_l
            .process_sample(&self.biquad_c_coefficients, input_l);
        input_r = self
            .biquad_c_r
            .process_sample(&self.biquad_c_coefficients, input_r);

        if wet != 1.0 {
            input_l += dry_l * (1.0 - wet);
            input_r += dry_r * (1.0 - wet);
        }

        frame[0] = input_l as f32;
        frame[1] = input_r as f32;
    }
}

impl Default for ReverbEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for ReverbEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![self.room_size.description(), self.wet.description()]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        self.channel_count = channel_count;
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "ReverbEffect only supports stereo I/O".to_string(),
            ));
        }
        self.room_size.set_sample_rate(sample_rate);
        self.wet.set_sample_rate(sample_rate);
        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        if self.room_size.value_need_ramp() || self.wet.value_need_ramp() {
            assert!(self.channel_count == 2);
            for frame in output.as_frames_mut::<2>() {
                let room_size = self.room_size.next_value() as f64;
                let wet = self.wet.next_value() as f64;
                let cutoff = (10000.0 - (room_size * wet * 3000.0)) as f32;
                let size = (room_size.powi(2) * 75.0) + 25.0;
                let depth_factor =
                    1.0 - (1.0 - (0.82 - (((1.0 - room_size) * 0.7) + (size * 0.002)))).powi(4);
                let blend = 0.955 - (size * 0.007);
                let regen = depth_factor * 0.5;
                // delays
                let predelay = self.update_delay_sizes(size);
                // filters
                self.update_filter_coefs(cutoff);
                // process
                self.process_frame(frame, blend, regen, predelay, wet);
            }
        } else {
            let room_size = self.room_size.target_value() as f64;
            let wet = self.wet.target_value() as f64;
            let cutoff = (10000.0 - (room_size * wet * 3000.0)) as f32;
            let size = (room_size.powi(2) * 75.0) + 25.0;
            let depth_factor =
                1.0 - (1.0 - (0.82 - (((1.0 - room_size) * 0.7) + (size * 0.002)))).powi(4);
            let blend = 0.955 - (size * 0.007);
            let regen = depth_factor * 0.5;
            // delays
            let predelay = self.update_delay_sizes(size);
            // filters
            self.update_filter_coefs(cutoff);
            // process
            assert!(self.channel_count == 2);
            for frame in output.as_frames_mut::<2>() {
                self.process_frame(frame, blend, regen, predelay, wet);
            }
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // 8 delay lines in feedback matrix: tail depends on longest delay line,
        // number of lines (8x), and regeneration factor
        let room_size = self.room_size.target_value() as f64;
        let size = (room_size.powi(2) * 75.0) + 25.0;
        let max_delay = (79.0 * size) as usize;
        let feedback = 1.0 - (1.0 - (0.82 - (((1.0 - room_size) * 0.7) + (size * 0.002)))).powi(4);
        if feedback >= 1.0 {
            Some(usize::MAX) // tail is infinitive
        } else {
            let decay_time_samples = if feedback == 0.0 {
                max_delay
            } else {
                const SILENCE: f64 = 0.001; // -60dB
                max_delay + (max_delay as f64 * SILENCE.log10() / feedback.log10()) as usize
            };
            Some(decay_time_samples)
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) -> Result<(), Error> {
        if let Some(message) = message.payload().downcast_ref::<ReverbEffectMessage>() {
            match message {
                ReverbEffectMessage::Reset => {
                    self.a.flush();
                    self.b.flush();
                    self.c.flush();
                    self.d.flush();
                    self.e.flush();
                    self.f.flush();
                    self.g.flush();
                    self.h.flush();
                    self.i.flush();
                    self.j.flush();
                    self.k.flush();
                    self.l.flush();
                    self.m.flush();
                }
            }
            Ok(())
        } else {
            Err(Error::ParameterError(
                "ReverbEffect: Invalid/unknown message payload".to_owned(),
            ))
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::ROOM_SIZE.id() => self.room_size.apply_update(value),
            _ if id == Self::WET.id() => self.wet.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        }
        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Reverb delay line with vibrato, feedback and interpolated read
struct ReverbDelayLine<const CHANNELS: usize> {
    // Interleaved delay line buffer
    buffer: Vec<[f64; CHANNELS]>,
    // Delay line counter
    count: usize,
    delay: usize,
    // Feedback
    feedback: [f64; CHANNELS],
    // Vibrato
    depth: f64,
    vib_phase: [f64; CHANNELS],
}

impl<const CHANNELS: usize> ReverbDelayLine<CHANNELS> {
    pub fn new(rng: &mut impl Rng, size: usize, depth: f64) -> Self {
        use std::f64::consts::PI;

        let mut vib_phase = [0.0; CHANNELS];
        for p in vib_phase.iter_mut() {
            *p = rng.random_range(0.0..2.0 * PI);
        }

        Self {
            buffer: vec![[0.0; CHANNELS]; size + 1],
            count: 1,
            delay: 1,
            depth,
            feedback: [0.0; CHANNELS],
            vib_phase,
        }
    }

    pub fn flush(&mut self) {
        self.buffer.fill([0.0; CHANNELS]);
    }

    pub fn get(&self, vib_depth: f64, blend: f64) -> [f64; CHANNELS] {
        let mut output = [0.0; CHANNELS];

        // Hint to optimizer: `self.delay` is clamped to `self.buffer.len() - 1` in the
        // setter, so read_1/2 are always valid here.
        assume!(unsafe: self.delay < self.buffer.len());

        #[allow(clippy::needless_range_loop)]
        for ch in 0..CHANNELS {
            let offset = (self.vib_phase[ch].sin() + 1.0) * vib_depth;
            let working = self.count as f64 + offset;
            let w_floor = working.floor();
            let w_frac = working - w_floor;
            let w_int = w_floor as usize;

            let mut read_1 = w_int;
            if read_1 > self.delay {
                read_1 -= self.delay + 1;
            }
            let mut read_2 = w_int + 1;
            if read_2 > self.delay {
                read_2 -= self.delay + 1;
            }

            let val1 = self.buffer[read_1][ch];
            let val2 = self.buffer[read_2][ch];

            let mut interpol = val1 * (1.0 - w_frac) + val2 * w_frac;
            interpol = (1.0 - blend) * interpol + (val1 * blend);
            output[ch] = interpol;
        }
        output
    }

    pub fn set(&mut self, values: [f64; CHANNELS]) {
        assume!(unsafe: self.count < self.buffer.len());
        let dest = &mut self.buffer[self.count];
        for ch in 0..CHANNELS {
            dest[ch] = values[ch] + self.feedback[ch];
        }
    }

    pub fn step(&mut self, speed: f64) {
        self.count += 1;
        if self.count > self.delay {
            self.count = 0;
        }
        for p in self.vib_phase.iter_mut() {
            *p += self.depth * speed;
        }
    }

    pub fn set_delay(&mut self, delay: usize) {
        debug_assert!(
            delay < self.buffer.len() - 1,
            "Delay must be < {} but is {}",
            self.buffer.len() - 1,
            delay
        );
        self.delay = delay.min(self.buffer.len() - 1);
    }
}
