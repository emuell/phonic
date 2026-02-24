use std::any::Any;

use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    effect::{Effect, EffectMessage, EffectMessagePayload, EffectTime},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, ParameterValueUpdate,
        SmoothedParameterValue,
    },
    utils::{
        buffer::InterleavedBufferMut,
        dsp::{
            delay::InterpolatedDelayLine,
            filters::{
                dc::{DcFilter, DcFilterMode},
                svf::{SvfFilter, SvfFilterCoefficients, SvfFilterType},
            },
            lfo::{Lfo, LfoWaveform},
        },
        smoothing::SigmoidSmoothedValue,
    },
    Error, Parameter, ParameterScaling,
};

// -------------------------------------------------------------------------------------------------

/// Message type for `DelayEffect`.
#[derive(Clone, Debug)]
#[allow(unused)]
pub enum DelayEffectMessage {
    /// Reset delay lines, filters and LFO state.
    Reset,
}

impl EffectMessage for DelayEffectMessage {
    fn effect_name(&self) -> &'static str {
        DelayEffect::EFFECT_NAME
    }
    fn payload(&self) -> &dyn Any {
        self
    }
}

// -------------------------------------------------------------------------------------------------

/// Stereo processing mode for `DelayEffect`.
#[derive(
    Default, Clone, Copy, PartialEq, strum::Display, strum::EnumString, strum::VariantNames,
)]
#[allow(unused)]
pub enum DelayEffectMode {
    /// Same delay on both channels (independent L/R processing).
    #[default]
    Stereo,
    /// Echoes alternate between left and right channels.
    #[strum(serialize = "Ping Pong")]
    PingPong,
}

// -------------------------------------------------------------------------------------------------

/// Filter type for the feedback filter in `DelayEffect`.
pub type DelayEffectFilterType = SvfFilterType;

// -------------------------------------------------------------------------------------------------

/// Soft-clipping saturation using a tanh approximation.
fn saturate(input: f64, drive: f32) -> f64 {
    if drive < 0.001 {
        return input;
    }
    let gain = 1.0 + drive as f64 * 4.0;
    let x = input * gain;
    let x2 = x * x;
    let output = x * (27.0 + x2) / (27.0 + 9.0 * x2);
    output / gain.sqrt()
}

// -------------------------------------------------------------------------------------------------

/// Stereo dub-style delay effect with filtered feedback, saturation and LFO modulation.
///
/// Supports two modes:
/// - **Stereo**: Independent delay on left and right channels
/// - **Ping Pong**: Echoes alternate between left and right
pub struct DelayEffect {
    sample_rate: u32,
    // Parameters
    mode: EnumParameterValue<DelayEffectMode>,
    delay_time: SmoothedParameterValue<SigmoidSmoothedValue>,
    feedback: SmoothedParameterValue,
    filter_cutoff: SmoothedParameterValue,
    filter_type: EnumParameterValue<DelayEffectFilterType>,
    drive: SmoothedParameterValue,
    wet_mix: SmoothedParameterValue,
    stereo_width: SmoothedParameterValue,
    lfo_rate: SmoothedParameterValue,
    lfo_shape: EnumParameterValue<LfoWaveform>,
    lfo_depth_time: SmoothedParameterValue,
    lfo_depth_feedback: SmoothedParameterValue,
    lfo_depth_filter: SmoothedParameterValue,
    // Runtime state
    delay_left: InterpolatedDelayLine<1>,
    delay_right: InterpolatedDelayLine<1>,
    lfo: Lfo,
    filter_coefficients: SvfFilterCoefficients,
    filter_left: SvfFilter,
    filter_right: SvfFilter,
    dc_left: DcFilter,
    dc_right: DcFilter,
    feedback_left: f32,
    feedback_right: f32,
}

impl DelayEffect {
    const MAX_DELAY_MS: f32 = 4000.0;
    const MAX_LFO_TIME_MOD_MS: f32 = 50.0;
    const FILTER_RESONANCE: f32 = 0.302; // Q = 0.7071

    pub const EFFECT_NAME: &str = "Delay";

    pub const MODE: EnumParameter = EnumParameter::new(
        FourCC(*b"mode"),
        "Mode",
        DelayEffectMode::VARIANTS,
        DelayEffectMode::Stereo as usize,
    );
    pub const DELAY_TIME: FloatParameter =
        FloatParameter::new(FourCC(*b"dlay"), "Delay", 1.0..=Self::MAX_DELAY_MS, 375.0)
            .with_unit("ms");
    pub const FEEDBACK: FloatParameter =
        FloatParameter::new(FourCC(*b"fdbk"), "Feedback", 0.0..=1.0, 0.5).with_unit("%");

    pub const FILTER_TYPE: EnumParameter = EnumParameter::new(
        FourCC(*b"ftyp"),
        "Filter Type",
        DelayEffectFilterType::VARIANTS,
        DelayEffectFilterType::Lowpass as usize,
    );
    pub const FILTER_CUTOFF: FloatParameter =
        FloatParameter::new(FourCC(*b"cuto"), "Filter Cutoff", 20.0..=20000.0, 6000.0)
            .with_scaling(ParameterScaling::Exponential(2.5))
            .with_unit("Hz");
    pub const DRIVE: FloatParameter =
        FloatParameter::new(FourCC(*b"driv"), "Drive", 0.0..=1.0, 0.0).with_unit("%");

    pub const WET_MIX: FloatParameter =
        FloatParameter::new(FourCC(*b"wet_"), "Wet", 0.0..=1.0, 0.5).with_unit("%");

    pub const STEREO_WIDTH: FloatParameter =
        FloatParameter::new(FourCC(*b"wdth"), "Width", 0.0..=1.0, 0.5).with_unit("%");

    pub const LFO_RATE: FloatParameter =
        FloatParameter::new(FourCC(*b"lfor"), "LFO Rate", 0.01..=10.0, 1.0)
            .with_scaling(ParameterScaling::Exponential(2.0))
            .with_unit("Hz");
    pub const LFO_SHAPE: EnumParameter = EnumParameter::new(
        FourCC(*b"lfos"),
        "LFO Shape",
        LfoWaveform::VARIANTS,
        LfoWaveform::Sine as usize,
    );
    pub const LFO_DEPTH_TIME: FloatParameter =
        FloatParameter::new(FourCC(*b"lfdt"), "LFO -> Time", -1.0..=1.0, 0.0).with_unit("%");
    pub const LFO_DEPTH_FEEDBACK: FloatParameter =
        FloatParameter::new(FourCC(*b"ldfb"), "LFO -> Feedback", -1.0..=1.0, 0.0).with_unit("%");
    pub const LFO_DEPTH_FILTER: FloatParameter =
        FloatParameter::new(FourCC(*b"lfdf"), "LFO -> Filter", -1.0..=1.0, 0.0).with_unit("%");

    /// Creates a new `DelayEffect` with default parameter values.
    pub fn new() -> Self {
        let to_string_percent = |v: f32| format!("{:.1}", v * 100.0);
        let from_string_percent = |v: &str| v.parse::<f32>().map(|f| f / 100.0).ok();

        Self {
            sample_rate: 0,

            mode: EnumParameterValue::from_description(Self::MODE),
            delay_time: SmoothedParameterValue::from_description(Self::DELAY_TIME)
                .with_smoother(SigmoidSmoothedValue::default().with_duration(44100 / 2)),
            feedback: SmoothedParameterValue::from_description(
                Self::FEEDBACK
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            filter_type: EnumParameterValue::from_description(Self::FILTER_TYPE),
            filter_cutoff: SmoothedParameterValue::from_description(Self::FILTER_CUTOFF),
            drive: SmoothedParameterValue::from_description(
                Self::DRIVE
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            wet_mix: SmoothedParameterValue::from_description(
                Self::WET_MIX
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            stereo_width: SmoothedParameterValue::from_description(
                Self::STEREO_WIDTH
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            lfo_rate: SmoothedParameterValue::from_description(Self::LFO_RATE),
            lfo_shape: EnumParameterValue::from_description(Self::LFO_SHAPE),
            lfo_depth_time: SmoothedParameterValue::from_description(
                Self::LFO_DEPTH_TIME
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            lfo_depth_feedback: SmoothedParameterValue::from_description(
                Self::LFO_DEPTH_FEEDBACK
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),
            lfo_depth_filter: SmoothedParameterValue::from_description(
                Self::LFO_DEPTH_FILTER
                    .clone()
                    .with_display(to_string_percent, from_string_percent),
            ),

            delay_left: InterpolatedDelayLine::default(),
            delay_right: InterpolatedDelayLine::default(),
            lfo: Lfo::default(),
            filter_coefficients: SvfFilterCoefficients::default(),
            filter_left: SvfFilter::default(),
            filter_right: SvfFilter::default(),
            dc_left: DcFilter::default(),
            dc_right: DcFilter::default(),
            feedback_left: 0.0,
            feedback_right: 0.0,
        }
    }

    fn reset(&mut self) {
        self.delay_left.flush();
        self.delay_right.flush();
        self.filter_left.reset();
        self.filter_right.reset();
        self.dc_left.reset();
        self.dc_right.reset();
        self.lfo.reset();
        self.feedback_left = 0.0;
        self.feedback_right = 0.0;
    }

    /// Process the feedback path: filter → saturate → DC block → clamp.
    #[inline]
    fn process_feedback(
        filter: &mut SvfFilter,
        coefficients: &SvfFilterCoefficients,
        dc: &mut DcFilter,
        delayed: f32,
        drive: f32,
    ) -> f32 {
        let filtered = filter.process_sample(coefficients, delayed as f64);
        let saturated = saturate(filtered, drive);
        let clean = dc.process_sample(saturated) as f32;
        clean.clamp(-4.0, 4.0)
    }
}

impl Default for DelayEffect {
    fn default() -> Self {
        Self::new()
    }
}

impl Effect for DelayEffect {
    fn name(&self) -> &'static str {
        Self::EFFECT_NAME
    }

    fn weight(&self) -> usize {
        3
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        vec![
            self.mode.description(),
            self.delay_time.description(),
            self.feedback.description(),
            self.filter_type.description(),
            self.filter_cutoff.description(),
            self.drive.description(),
            self.wet_mix.description(),
            self.stereo_width.description(),
            self.lfo_rate.description(),
            self.lfo_shape.description(),
            self.lfo_depth_time.description(),
            self.lfo_depth_feedback.description(),
            self.lfo_depth_filter.description(),
        ]
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        _max_frames: usize,
    ) -> Result<(), Error> {
        self.sample_rate = sample_rate;
        if channel_count != 2 {
            return Err(Error::ParameterError(
                "DelayEffect only supports stereo I/O".to_owned(),
            ));
        }

        // Set sample rates on all smoothed parameters
        self.delay_time.set_sample_rate(sample_rate);
        self.feedback.set_sample_rate(sample_rate);
        self.filter_cutoff.set_sample_rate(sample_rate);
        self.drive.set_sample_rate(sample_rate);
        self.wet_mix.set_sample_rate(sample_rate);
        self.stereo_width.set_sample_rate(sample_rate);
        self.lfo_rate.set_sample_rate(sample_rate);
        self.lfo_depth_time.set_sample_rate(sample_rate);
        self.lfo_depth_feedback.set_sample_rate(sample_rate);
        self.lfo_depth_filter.set_sample_rate(sample_rate);

        // Allocate delay lines: max delay + max LFO modulation + margin
        let max_delay_samples =
            ((Self::MAX_DELAY_MS + Self::MAX_LFO_TIME_MOD_MS) * sample_rate as f32 / 1000.0).ceil()
                as usize;
        self.delay_left = InterpolatedDelayLine::new(max_delay_samples + 4);
        self.delay_right = InterpolatedDelayLine::new(max_delay_samples + 4);

        // Init filter
        self.filter_coefficients = SvfFilterCoefficients::new(
            self.filter_type.value(),
            sample_rate,
            self.filter_cutoff.target_value(),
            Self::FILTER_RESONANCE,
        )?;

        // Init LFO
        self.lfo = Lfo::new(
            sample_rate,
            self.lfo_rate.target_value() as f64,
            self.lfo_shape.value(),
        );

        // Init DC filters
        self.dc_left = DcFilter::new(sample_rate, DcFilterMode::Default);
        self.dc_right = DcFilter::new(sample_rate, DcFilterMode::Default);

        self.feedback_left = 0.0;
        self.feedback_right = 0.0;

        Ok(())
    }

    fn process(&mut self, mut output: &mut [f32], _time: &EffectTime) {
        let sample_rate = self.sample_rate as f32;
        let mode = self.mode.value();

        for frame in output.as_frames_mut::<2>() {
            let left_input = frame[0];
            let right_input = frame[1];

            // LFO
            let lfo_val = self.lfo.run();

            // Update LFO rate if ramping
            if self.lfo_rate.value_need_ramp() {
                let rate = self.lfo_rate.next_value();
                self.lfo.set_rate(self.sample_rate, rate as f64);
            }

            // Compute modulated delay time
            let base_delay_ms = self.delay_time.next_value();
            let time_mod_ms =
                lfo_val * self.lfo_depth_time.next_value() * Self::MAX_LFO_TIME_MOD_MS;
            let delay_ms = (base_delay_ms + time_mod_ms).max(1.0);
            let delay_samples = delay_ms * 0.001 * sample_rate;

            // Compute modulated filter cutoff (bipolar: negative depth inverts modulation)
            let filter_depth = self.lfo_depth_filter.next_value();
            let filter_mod = 2.0_f32.powf(lfo_val * filter_depth * 2.0);

            // Update filter coefficients (shared between L/R)
            let cutoff = (self.filter_cutoff.next_value() * filter_mod)
                .clamp(20.0, self.sample_rate as f32 / 2.0);
            let _ = self.filter_coefficients.set(
                self.filter_type.value(),
                self.sample_rate,
                cutoff,
                Self::FILTER_RESONANCE,
            );

            // Read parameters
            let base_feedback = self.feedback.next_value();
            let feedback_depth = self.lfo_depth_feedback.next_value();
            let feedback = (base_feedback + lfo_val * feedback_depth * (1.0 - base_feedback.abs()))
                .clamp(0.0, 0.999);
            let drive = self.drive.next_value();
            let wet_mix = self.wet_mix.next_value();
            let width = self.stereo_width.next_value();

            let (wet_l, wet_r) = match mode {
                DelayEffectMode::Stereo => {
                    // Independent delay on each channel
                    let l_in = left_input + self.feedback_left * feedback;
                    let delayed_l = self.delay_left.process([l_in], 0.0, delay_samples)[0];
                    let clean_l = Self::process_feedback(
                        &mut self.filter_left,
                        &self.filter_coefficients,
                        &mut self.dc_left,
                        delayed_l,
                        drive,
                    );
                    self.feedback_left = clean_l;

                    let r_in = right_input + self.feedback_right * feedback;
                    let delayed_r = self.delay_right.process([r_in], 0.0, delay_samples)[0];
                    let clean_r = Self::process_feedback(
                        &mut self.filter_right,
                        &self.filter_coefficients,
                        &mut self.dc_right,
                        delayed_r,
                        drive,
                    );
                    self.feedback_right = clean_r;

                    (clean_l, clean_r)
                }
                DelayEffectMode::PingPong => {
                    // Mono input -> left delay; left feeds right, right feeds left
                    let mono_in = (left_input + right_input) * 0.5;

                    // Left: input + feedback from RIGHT
                    let l_in = mono_in + self.feedback_right * feedback;
                    let delayed_l = self.delay_left.process([l_in], 0.0, delay_samples)[0];
                    let clean_l = Self::process_feedback(
                        &mut self.filter_left,
                        &self.filter_coefficients,
                        &mut self.dc_left,
                        delayed_l,
                        drive,
                    );

                    // Right: feedback from LEFT only (no direct input)
                    let r_in = self.feedback_left * feedback;
                    let delayed_r = self.delay_right.process([r_in], 0.0, delay_samples)[0];
                    let clean_r = Self::process_feedback(
                        &mut self.filter_right,
                        &self.filter_coefficients,
                        &mut self.dc_right,
                        delayed_r,
                        drive,
                    );

                    self.feedback_left = clean_l;
                    self.feedback_right = clean_r;

                    (clean_l, clean_r)
                }
            };

            // Mix dry/wet: both dry and wet are 1.0 when mix is 0.5, with dry decreasing to 0
            // above this value and wet decreasing to 0 below it
            let dry_gain = ((1.0 - wet_mix) * 2.0).min(1.0);
            let wet_gain = (wet_mix * 2.0).min(1.0);
            let out_l = left_input * dry_gain + wet_l * wet_gain;
            let out_r = right_input * dry_gain + wet_r * wet_gain;

            // Stereo width: crossfade between mono and full stereo
            let mid = (out_l + out_r) * 0.5;
            let side = (out_l - out_r) * 0.5;
            frame[0] = mid + side * width;
            frame[1] = mid - side * width;
        }
    }

    fn process_tail(&self) -> Option<usize> {
        // No way to calc resonance behavior with drive involed. Let mixer check for silence...
        if self.drive.target_value() > 0.0 {
            return None;
        }

        let delay_ms = (self.delay_time.target_value() + Self::MAX_LFO_TIME_MOD_MS) as f64;
        let feedback = self.feedback.target_value().abs() as f64;

        if feedback >= 0.9999 {
            Some(usize::MAX)
        } else if feedback < 0.001 {
            Some((delay_ms * self.sample_rate as f64 / 1000.0).ceil() as usize)
        } else {
            const SILENCE: f64 = 0.001; // -60dB threshold
            let delay_samples = delay_ms * self.sample_rate as f64 / 1000.0;
            let decay_samples = delay_samples + delay_samples * SILENCE.log10() / feedback.log10();
            Some((decay_samples.ceil() as usize).max(1))
        }
    }

    fn process_message(&mut self, message: &EffectMessagePayload) -> Result<(), Error> {
        if let Some(message) = message.payload().downcast_ref::<DelayEffectMessage>() {
            match message {
                DelayEffectMessage::Reset => self.reset(),
            }
            Ok(())
        } else {
            Err(Error::ParameterError(
                "DelayEffect: Invalid/unknown message payload".to_owned(),
            ))
        }
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::MODE.id() => self.mode.apply_update(value),
            _ if id == Self::DELAY_TIME.id() => self.delay_time.apply_update(value),
            _ if id == Self::FEEDBACK.id() => self.feedback.apply_update(value),
            _ if id == Self::FILTER_TYPE.id() => self.filter_type.apply_update(value),
            _ if id == Self::FILTER_CUTOFF.id() => self.filter_cutoff.apply_update(value),
            _ if id == Self::DRIVE.id() => self.drive.apply_update(value),
            _ if id == Self::WET_MIX.id() => self.wet_mix.apply_update(value),
            _ if id == Self::STEREO_WIDTH.id() => self.stereo_width.apply_update(value),
            _ if id == Self::LFO_RATE.id() => self.lfo_rate.apply_update(value),
            _ if id == Self::LFO_SHAPE.id() => {
                self.lfo_shape.apply_update(value);
                self.lfo.set_waveform(self.lfo_shape.value());
            }
            _ if id == Self::LFO_DEPTH_TIME.id() => self.lfo_depth_time.apply_update(value),
            _ if id == Self::LFO_DEPTH_FEEDBACK.id() => self.lfo_depth_feedback.apply_update(value),
            _ if id == Self::LFO_DEPTH_FILTER.id() => self.lfo_depth_filter.apply_update(value),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown parameter: '{id}' for effect '{}'",
                    self.name()
                )))
            }
        };
        Ok(())
    }
}
