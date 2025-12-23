use four_cc::FourCC;
use rand::Rng;
use std::any::Any;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{FloatParameter, ParameterValueUpdate, SmoothedParameterValue},
    utils::{
        buffer::InterleavedBufferMut,
        dsp::filters::biquad::{BiquadFilter, BiquadFilterCoefficients, BiquadFilterType},
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

    a_al: Vec<f64>,
    a_ar: Vec<f64>,
    a_bl: Vec<f64>,
    a_br: Vec<f64>,
    a_cl: Vec<f64>,
    a_cr: Vec<f64>,
    a_dl: Vec<f64>,
    a_dr: Vec<f64>,
    a_el: Vec<f64>,
    a_er: Vec<f64>,
    a_fl: Vec<f64>,
    a_fr: Vec<f64>,
    a_gl: Vec<f64>,
    a_gr: Vec<f64>,
    a_hl: Vec<f64>,
    a_hr: Vec<f64>,
    a_il: Vec<f64>,
    a_ir: Vec<f64>,
    a_jl: Vec<f64>,
    a_jr: Vec<f64>,
    a_kl: Vec<f64>,
    a_kr: Vec<f64>,
    a_ll: Vec<f64>,
    a_lr: Vec<f64>,
    a_ml: Vec<f64>,
    a_mr: Vec<f64>,

    count_a: usize,
    delay_a: usize,
    count_b: usize,
    delay_b: usize,
    count_c: usize,
    delay_c: usize,
    count_d: usize,
    delay_d: usize,
    count_e: usize,
    delay_e: usize,
    count_f: usize,
    delay_f: usize,
    count_g: usize,
    delay_g: usize,
    count_h: usize,
    delay_h: usize,
    count_i: usize,
    delay_i: usize,
    count_j: usize,
    delay_j: usize,
    count_k: usize,
    delay_k: usize,
    count_l: usize,
    delay_l: usize,
    count_m: usize,
    delay_m: usize,

    feedback_al: f64,
    vib_al: f64,
    depth_a: f64,
    feedback_bl: f64,
    vib_bl: f64,
    depth_b: f64,
    feedback_cl: f64,
    vib_cl: f64,
    depth_c: f64,
    feedback_dl: f64,
    vib_dl: f64,
    depth_d: f64,
    feedback_el: f64,
    vib_el: f64,
    depth_e: f64,
    feedback_fl: f64,
    vib_fl: f64,
    depth_f: f64,
    feedback_gl: f64,
    vib_gl: f64,
    depth_g: f64,
    feedback_hl: f64,
    vib_hl: f64,
    depth_h: f64,

    feedback_ar: f64,
    vib_ar: f64,
    feedback_br: f64,
    vib_br: f64,
    feedback_cr: f64,
    vib_cr: f64,
    feedback_dr: f64,
    vib_dr: f64,
    feedback_er: f64,
    vib_er: f64,
    feedback_fr: f64,
    vib_fr: f64,
    feedback_gr: f64,
    vib_gr: f64,
    feedback_hr: f64,
    vib_hr: f64,

    fpd_l: u32,
    fpd_r: u32,
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

        use std::f64::consts::PI;

        const A_AL_SIZE: usize = 8111;
        const A_BL_SIZE: usize = 7511;
        const A_CL_SIZE: usize = 7311;
        const A_DL_SIZE: usize = 6911;
        const A_EL_SIZE: usize = 6311;
        const A_FL_SIZE: usize = 6111;
        const A_GL_SIZE: usize = 5511;
        const A_HL_SIZE: usize = 4911;
        const A_IL_SIZE: usize = 4511;
        const A_JL_SIZE: usize = 4311;
        const A_KL_SIZE: usize = 3911;
        const A_LL_SIZE: usize = 3311;
        const A_ML_SIZE: usize = 3111;

        let to_string_percent = |v: f32| format!("{:.2}", v * 100.0);
        let from_string_percent = |v: &str| v.parse::<f32>().map(|f| f / 100.0).ok();

        Self {
            sample_rate: 0,
            channel_count: 0,
            room_size: SmoothedParameterValue::from_description(
                Self::ROOM_SIZE
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
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
            a_al: vec![0.0; A_AL_SIZE],
            a_ar: vec![0.0; A_AL_SIZE],
            a_bl: vec![0.0; A_BL_SIZE],
            a_br: vec![0.0; A_BL_SIZE],
            a_cl: vec![0.0; A_CL_SIZE],
            a_cr: vec![0.0; A_CL_SIZE],
            a_dl: vec![0.0; A_DL_SIZE],
            a_dr: vec![0.0; A_DL_SIZE],
            a_el: vec![0.0; A_EL_SIZE],
            a_er: vec![0.0; A_EL_SIZE],
            a_fl: vec![0.0; A_FL_SIZE],
            a_fr: vec![0.0; A_FL_SIZE],
            a_gl: vec![0.0; A_GL_SIZE],
            a_gr: vec![0.0; A_GL_SIZE],
            a_hl: vec![0.0; A_HL_SIZE],
            a_hr: vec![0.0; A_HL_SIZE],
            a_il: vec![0.0; A_IL_SIZE],
            a_ir: vec![0.0; A_IL_SIZE],
            a_jl: vec![0.0; A_JL_SIZE],
            a_jr: vec![0.0; A_JL_SIZE],
            a_kl: vec![0.0; A_KL_SIZE],
            a_kr: vec![0.0; A_KL_SIZE],
            a_ll: vec![0.0; A_LL_SIZE],
            a_lr: vec![0.0; A_LL_SIZE],
            a_ml: vec![0.0; A_ML_SIZE],
            a_mr: vec![0.0; A_ML_SIZE],
            count_a: 1,
            delay_a: 79,
            count_b: 1,
            delay_b: 73,
            count_c: 1,
            delay_c: 71,
            count_d: 1,
            delay_d: 67,
            count_e: 1,
            delay_e: 61,
            count_f: 1,
            delay_f: 59,
            count_g: 1,
            delay_g: 53,
            count_h: 1,
            delay_h: 47,
            count_i: 1,
            delay_i: 43,
            count_j: 1,
            delay_j: 41,
            count_k: 1,
            delay_k: 37,
            count_l: 1,
            delay_l: 31,
            count_m: 1,
            delay_m: 29,
            feedback_al: 0.0,
            feedback_ar: 0.0,
            feedback_bl: 0.0,
            feedback_br: 0.0,
            feedback_cl: 0.0,
            feedback_cr: 0.0,
            feedback_dl: 0.0,
            feedback_dr: 0.0,
            feedback_el: 0.0,
            feedback_er: 0.0,
            feedback_fl: 0.0,
            feedback_fr: 0.0,
            feedback_gl: 0.0,
            feedback_gr: 0.0,
            feedback_hl: 0.0,
            feedback_hr: 0.0,
            depth_a: 0.003251,
            depth_b: 0.002999,
            depth_c: 0.002917,
            depth_d: 0.002749,
            depth_e: 0.002503,
            depth_f: 0.002423,
            depth_g: 0.002146,
            depth_h: 0.002088,
            vib_al: rng.random_range(0.0..2.0 * PI),
            vib_ar: rng.random_range(0.0..2.0 * PI),
            vib_bl: rng.random_range(0.0..2.0 * PI),
            vib_br: rng.random_range(0.0..2.0 * PI),
            vib_cl: rng.random_range(0.0..2.0 * PI),
            vib_cr: rng.random_range(0.0..2.0 * PI),
            vib_dl: rng.random_range(0.0..2.0 * PI),
            vib_dr: rng.random_range(0.0..2.0 * PI),
            vib_el: rng.random_range(0.0..2.0 * PI),
            vib_er: rng.random_range(0.0..2.0 * PI),
            vib_fl: rng.random_range(0.0..2.0 * PI),
            vib_fr: rng.random_range(0.0..2.0 * PI),
            vib_gl: rng.random_range(0.0..2.0 * PI),
            vib_gr: rng.random_range(0.0..2.0 * PI),
            vib_hl: rng.random_range(0.0..2.0 * PI),
            vib_hr: rng.random_range(0.0..2.0 * PI),
            fpd_l,
            fpd_r,
        }
    }

    /// Creates a new `ReverbEffect` with the given parameters.
    pub fn with_parameters(room_size: f32, wet: f32) -> Self {
        let mut reverb = Self::new();
        reverb.room_size.init_value(room_size);
        reverb.wet.init_value(wet);
        reverb
    }

    fn reset(&mut self) {
        for v in [
            &mut self.a_al,
            &mut self.a_ar,
            &mut self.a_bl,
            &mut self.a_br,
            &mut self.a_cl,
            &mut self.a_cr,
            &mut self.a_dl,
            &mut self.a_dr,
            &mut self.a_el,
            &mut self.a_er,
            &mut self.a_fl,
            &mut self.a_fr,
            &mut self.a_gl,
            &mut self.a_gr,
            &mut self.a_hl,
            &mut self.a_hr,
            &mut self.a_il,
            &mut self.a_ir,
            &mut self.a_jl,
            &mut self.a_jr,
            &mut self.a_kl,
            &mut self.a_kr,
            &mut self.a_ll,
            &mut self.a_lr,
            &mut self.a_ml,
            &mut self.a_mr,
        ] {
            v.fill(0.0);
        }
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
        let room_size = self.room_size.next_value() as f64;
        let wet = self.wet.next_value() as f64;
        let vib_speed = 0.1;
        let vib_depth = 7.0;
        let size = (room_size.powi(2) * 75.0) + 25.0;
        let depth_factor =
            1.0 - (1.0 - (0.82 - (((1.0 - room_size) * 0.7) + (size * 0.002)))).powi(4);
        let blend = 0.955 - (size * 0.007);
        let regen = depth_factor * 0.5;

        self.delay_a = (79.0 * size) as usize;
        self.delay_b = (73.0 * size) as usize;
        self.delay_c = (71.0 * size) as usize;
        self.delay_d = (67.0 * size) as usize;
        self.delay_e = (61.0 * size) as usize;
        self.delay_f = (59.0 * size) as usize;
        self.delay_g = (53.0 * size) as usize;
        self.delay_h = (47.0 * size) as usize;
        self.delay_i = (43.0 * size) as usize;
        self.delay_j = (41.0 * size) as usize;
        self.delay_k = (37.0 * size) as usize;
        self.delay_l = (31.0 * size) as usize;
        self.delay_m = (29.0 * size) as usize;

        let cutoff = (10000.0 - (room_size * wet * 3000.0)) as f32;
        self.biquad_a_coefficients
            .set(
                BiquadFilterType::Lowpass,
                self.sample_rate,
                cutoff,
                1.618034,
                0.0,
            )
            .unwrap();
        self.biquad_b_coefficients
            .set(
                BiquadFilterType::Lowpass,
                self.sample_rate,
                cutoff,
                0.618034,
                0.0,
            )
            .unwrap();
        self.biquad_c_coefficients
            .set(
                //
                BiquadFilterType::Lowpass,
                self.sample_rate,
                cutoff,
                0.5,
                0.0,
            )
            .unwrap();

        assert!(self.channel_count == 2);
        for frame in output.as_frames_mut::<2>() {
            let mut input_sample_l = frame[0] as f64;
            let mut input_sample_r = frame[1] as f64;

            if input_sample_l.abs() < 1.18e-23 {
                input_sample_l = self.fpd_l as f64 * 1.18e-17;
            }
            if input_sample_r.abs() < 1.18e-23 {
                input_sample_r = self.fpd_r as f64 * 1.18e-17;
            }
            let dry_sample_l = input_sample_l;
            let dry_sample_r = input_sample_r;

            // Predelay
            self.a_ml[self.count_m] = input_sample_l;
            self.a_mr[self.count_m] = input_sample_r;
            self.count_m += 1;
            if self.count_m > self.delay_m {
                self.count_m = 0;
            }
            input_sample_l = self.a_ml[self.count_m];
            input_sample_r = self.a_mr[self.count_m];

            // Biquad A
            input_sample_l = self
                .biquad_a_l
                .process_sample(&self.biquad_a_coefficients, input_sample_l);
            input_sample_r = self
                .biquad_a_r
                .process_sample(&self.biquad_a_coefficients, input_sample_r);

            input_sample_l = input_sample_l.sin();
            input_sample_r = input_sample_r.sin();

            // Allpasses
            let mut allpass_il = input_sample_l;
            let mut allpass_ir = input_sample_r;
            let mut allpass_jl = input_sample_l;
            let mut allpass_jr = input_sample_r;
            let mut allpass_kl = input_sample_l;
            let mut allpass_kr = input_sample_r;
            let mut allpass_ll = input_sample_l;
            let mut allpass_lr = input_sample_r;

            let mut ap_temp = self.count_i + 1;
            if ap_temp > self.delay_i {
                ap_temp = 0;
            }
            allpass_il -= self.a_il[ap_temp] * 0.5;
            self.a_il[self.count_i] = allpass_il;
            allpass_il *= 0.5;
            allpass_ir -= self.a_ir[ap_temp] * 0.5;
            self.a_ir[self.count_i] = allpass_ir;
            allpass_ir *= 0.5;
            self.count_i += 1;
            if self.count_i > self.delay_i {
                self.count_i = 0;
            }
            allpass_il += self.a_il[self.count_i];
            allpass_ir += self.a_ir[self.count_i];

            ap_temp = self.count_j + 1;
            if ap_temp > self.delay_j {
                ap_temp = 0;
            }
            allpass_jl -= self.a_jl[ap_temp] * 0.5;
            self.a_jl[self.count_j] = allpass_jl;
            allpass_jl *= 0.5;
            allpass_jr -= self.a_jr[ap_temp] * 0.5;
            self.a_jr[self.count_j] = allpass_jr;
            allpass_jr *= 0.5;
            self.count_j += 1;
            if self.count_j > self.delay_j {
                self.count_j = 0;
            }
            allpass_jl += self.a_jl[self.count_j];
            allpass_jr += self.a_jr[self.count_j];

            ap_temp = self.count_k + 1;
            if ap_temp > self.delay_k {
                ap_temp = 0;
            }
            allpass_kl -= self.a_kl[ap_temp] * 0.5;
            self.a_kl[self.count_k] = allpass_kl;
            allpass_kl *= 0.5;
            allpass_kr -= self.a_kr[ap_temp] * 0.5;
            self.a_kr[self.count_k] = allpass_kr;
            allpass_kr *= 0.5;
            self.count_k += 1;
            if self.count_k > self.delay_k {
                self.count_k = 0;
            }
            allpass_kl += self.a_kl[self.count_k];
            allpass_kr += self.a_kr[self.count_k];

            ap_temp = self.count_l + 1;
            if ap_temp > self.delay_l {
                ap_temp = 0;
            }
            allpass_ll -= self.a_ll[ap_temp] * 0.5;
            self.a_ll[self.count_l] = allpass_ll;
            allpass_ll *= 0.5;
            allpass_lr -= self.a_lr[ap_temp] * 0.5;
            self.a_lr[self.count_l] = allpass_lr;
            allpass_lr *= 0.5;
            self.count_l += 1;
            if self.count_l > self.delay_l {
                self.count_l = 0;
            }
            allpass_ll += self.a_ll[self.count_l];
            allpass_lr += self.a_lr[self.count_l];

            // Householder Matrix
            self.a_al[self.count_a] = allpass_ll + self.feedback_al;
            self.a_ar[self.count_a] = allpass_lr + self.feedback_ar;
            self.a_bl[self.count_b] = allpass_kl + self.feedback_bl;
            self.a_br[self.count_b] = allpass_kr + self.feedback_br;
            self.a_cl[self.count_c] = allpass_jl + self.feedback_cl;
            self.a_cr[self.count_c] = allpass_jr + self.feedback_cr;
            self.a_dl[self.count_d] = allpass_il + self.feedback_dl;
            self.a_dr[self.count_d] = allpass_ir + self.feedback_dr;
            self.a_el[self.count_e] = allpass_il + self.feedback_el;
            self.a_er[self.count_e] = allpass_ir + self.feedback_er;
            self.a_fl[self.count_f] = allpass_jl + self.feedback_fl;
            self.a_fr[self.count_f] = allpass_jr + self.feedback_fr;
            self.a_gl[self.count_g] = allpass_kl + self.feedback_gl;
            self.a_gr[self.count_g] = allpass_kr + self.feedback_gr;
            self.a_hl[self.count_h] = allpass_ll + self.feedback_hl;
            self.a_hr[self.count_h] = allpass_lr + self.feedback_hr;

            self.count_a += 1;
            if self.count_a > self.delay_a {
                self.count_a = 0;
            }
            self.count_b += 1;
            if self.count_b > self.delay_b {
                self.count_b = 0;
            }
            self.count_c += 1;
            if self.count_c > self.delay_c {
                self.count_c = 0;
            }
            self.count_d += 1;
            if self.count_d > self.delay_d {
                self.count_d = 0;
            }
            self.count_e += 1;
            if self.count_e > self.delay_e {
                self.count_e = 0;
            }
            self.count_f += 1;
            if self.count_f > self.delay_f {
                self.count_f = 0;
            }
            self.count_g += 1;
            if self.count_g > self.delay_g {
                self.count_g = 0;
            }
            self.count_h += 1;
            if self.count_h > self.delay_h {
                self.count_h = 0;
            }

            // Vibrato
            self.vib_al += self.depth_a * vib_speed;
            self.vib_ar += self.depth_a * vib_speed;
            self.vib_bl += self.depth_b * vib_speed;
            self.vib_br += self.depth_b * vib_speed;
            self.vib_cl += self.depth_c * vib_speed;
            self.vib_cr += self.depth_c * vib_speed;
            self.vib_dl += self.depth_d * vib_speed;
            self.vib_dr += self.depth_d * vib_speed;
            self.vib_el += self.depth_e * vib_speed;
            self.vib_er += self.depth_e * vib_speed;
            self.vib_fl += self.depth_f * vib_speed;
            self.vib_fr += self.depth_f * vib_speed;
            self.vib_gl += self.depth_g * vib_speed;
            self.vib_gr += self.depth_g * vib_speed;
            self.vib_hl += self.depth_h * vib_speed;
            self.vib_hr += self.depth_h * vib_speed;

            let get_offset = |vib: f64| (vib.sin() + 1.0) * vib_depth;
            let offset_al = get_offset(self.vib_al);
            let offset_ar = get_offset(self.vib_ar);
            let offset_bl = get_offset(self.vib_bl);
            let offset_br = get_offset(self.vib_br);
            let offset_cl = get_offset(self.vib_cl);
            let offset_cr = get_offset(self.vib_cr);
            let offset_dl = get_offset(self.vib_dl);
            let offset_dr = get_offset(self.vib_dr);
            let offset_el = get_offset(self.vib_el);
            let offset_er = get_offset(self.vib_er);
            let offset_fl = get_offset(self.vib_fl);
            let offset_fr = get_offset(self.vib_fr);
            let offset_gl = get_offset(self.vib_gl);
            let offset_gr = get_offset(self.vib_gr);
            let offset_hl = get_offset(self.vib_hl);
            let offset_hr = get_offset(self.vib_hr);

            let get_interpol = |buf: &[f64], count: usize, delay: usize, offset: f64| {
                let working = count as f64 + offset;
                let w_floor = working.floor();
                let w_frac = working - w_floor;
                let w_int = w_floor as usize;

                let mut read_1 = w_int;
                if read_1 > delay {
                    read_1 -= delay + 1;
                }
                let mut read_2 = w_int + 1;
                if read_2 > delay {
                    read_2 -= delay + 1;
                }

                let val1 = buf[read_1];
                let val2 = buf[read_2];
                let mut interpol = val1 * (1.0 - w_frac) + val2 * w_frac;
                interpol = (1.0 - blend) * interpol + (buf[read_1] * blend);
                interpol
            };

            let interpol_al = get_interpol(&self.a_al, self.count_a, self.delay_a, offset_al);
            let interpol_bl = get_interpol(&self.a_bl, self.count_b, self.delay_b, offset_bl);
            let interpol_cl = get_interpol(&self.a_cl, self.count_c, self.delay_c, offset_cl);
            let interpol_dl = get_interpol(&self.a_dl, self.count_d, self.delay_d, offset_dl);
            let interpol_el = get_interpol(&self.a_el, self.count_e, self.delay_e, offset_el);
            let interpol_fl = get_interpol(&self.a_fl, self.count_f, self.delay_f, offset_fl);
            let interpol_gl = get_interpol(&self.a_gl, self.count_g, self.delay_g, offset_gl);
            let interpol_hl = get_interpol(&self.a_hl, self.count_h, self.delay_h, offset_hl);

            let interpol_ar = get_interpol(&self.a_ar, self.count_a, self.delay_a, offset_ar);
            let interpol_br = get_interpol(&self.a_br, self.count_b, self.delay_b, offset_br);
            let interpol_cr = get_interpol(&self.a_cr, self.count_c, self.delay_c, offset_cr);
            let interpol_dr = get_interpol(&self.a_dr, self.count_d, self.delay_d, offset_dr);
            let interpol_er = get_interpol(&self.a_er, self.count_e, self.delay_e, offset_er);
            let interpol_fr = get_interpol(&self.a_fr, self.count_f, self.delay_f, offset_fr);
            let interpol_gr = get_interpol(&self.a_gr, self.count_g, self.delay_g, offset_gr);
            let interpol_hr = get_interpol(&self.a_hr, self.count_h, self.delay_h, offset_hr);

            // Feedback
            self.feedback_al = (interpol_al - (interpol_bl + interpol_cl + interpol_dl)) * regen;
            self.feedback_bl = (interpol_bl - (interpol_al + interpol_cl + interpol_dl)) * regen;
            self.feedback_cl = (interpol_cl - (interpol_al + interpol_bl + interpol_dl)) * regen;
            self.feedback_dl = (interpol_dl - (interpol_al + interpol_bl + interpol_cl)) * regen;
            self.feedback_el = (interpol_el - (interpol_fl + interpol_gl + interpol_hl)) * regen;
            self.feedback_fl = (interpol_fl - (interpol_el + interpol_gl + interpol_hl)) * regen;
            self.feedback_gl = (interpol_gl - (interpol_el + interpol_fl + interpol_hl)) * regen;
            self.feedback_hl = (interpol_hl - (interpol_el + interpol_fl + interpol_gl)) * regen;

            self.feedback_ar = (interpol_ar - (interpol_br + interpol_cr + interpol_dr)) * regen;
            self.feedback_br = (interpol_br - (interpol_ar + interpol_cr + interpol_dr)) * regen;
            self.feedback_cr = (interpol_cr - (interpol_ar + interpol_br + interpol_dr)) * regen;
            self.feedback_dr = (interpol_dr - (interpol_ar + interpol_br + interpol_cr)) * regen;
            self.feedback_er = (interpol_er - (interpol_fr + interpol_gr + interpol_hr)) * regen;
            self.feedback_fr = (interpol_fr - (interpol_er + interpol_gr + interpol_hr)) * regen;
            self.feedback_gr = (interpol_gr - (interpol_er + interpol_fr + interpol_hr)) * regen;
            self.feedback_hr = (interpol_hr - (interpol_er + interpol_fr + interpol_gr)) * regen;

            input_sample_l = (interpol_al
                + interpol_bl
                + interpol_cl
                + interpol_dl
                + interpol_el
                + interpol_fl
                + interpol_gl
                + interpol_hl)
                / 8.0;
            input_sample_r = (interpol_ar
                + interpol_br
                + interpol_cr
                + interpol_dr
                + interpol_er
                + interpol_fr
                + interpol_gr
                + interpol_hr)
                / 8.0;

            // Biquad B
            input_sample_l = self
                .biquad_b_l
                .process_sample(&self.biquad_b_coefficients, input_sample_l);
            input_sample_r = self
                .biquad_b_r
                .process_sample(&self.biquad_b_coefficients, input_sample_r);

            input_sample_l = input_sample_l.clamp(-1.0, 1.0);
            input_sample_r = input_sample_r.clamp(-1.0, 1.0);

            input_sample_l = input_sample_l.asin();
            input_sample_r = input_sample_r.asin();

            // Biquad C
            input_sample_l = self
                .biquad_c_l
                .process_sample(&self.biquad_c_coefficients, input_sample_l);
            input_sample_r = self
                .biquad_c_r
                .process_sample(&self.biquad_c_coefficients, input_sample_r);

            input_sample_l = dry_sample_l + input_sample_l * wet;
            input_sample_r = dry_sample_r + input_sample_r * wet;

            frame[0] = input_sample_l as f32;
            frame[1] = input_sample_r as f32;
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
                ReverbEffectMessage::Reset => self.reset(),
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
