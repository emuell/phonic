use std::{
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    generator::{
        Generator, GeneratorPlaybackEvent, GeneratorPlaybackMessage, GeneratorPlaybackOptions,
    },
    modulation::{ModulationConfig, ModulationSource, ModulationTarget},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, IntegerParameter, Parameter,
        ParameterScaling, ParameterValueUpdate,
    },
    source::{
        file::preloaded::PreloadedFileSource, mixed::MixedSource, unique_source_id, Source,
        SourceTime,
    },
    sources::PreloadedFileBuffer,
    utils::{
        ahdsr::AhdsrParameters,
        buffer::{add_buffers, clear_buffer},
        db_to_linear,
        dsp::lfo::LfoWaveform,
        linear_to_db,
    },
    Error, FilePlaybackOptions, FileSource, NotePlaybackId, PlaybackId, PlaybackStatusContext,
    PlaybackStatusEvent, ResamplingQuality,
};

// -------------------------------------------------------------------------------------------------

mod granular;
mod modulation;
mod voice;

use modulation::SamplerModulationState;
use voice::SamplerVoice;

pub use granular::{GrainOverlapMode, GrainPlaybackDirection, GrainWindowMode, GranularParameters};

// -------------------------------------------------------------------------------------------------

/// Basic sampler which plays a single audio file with optional AHDSR envelope and/or
/// granular playback on a predefined number of voices.
///
/// AHDSR and granular parameters can be automated.
pub struct Sampler {
    playback_id: PlaybackId,
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    file_path: Arc<String>,
    active_voices: usize,
    voices: Vec<SamplerVoice>,
    base_transpose: i32,
    base_finetune: i32,
    base_volume: f32,
    base_panning: f32,
    envelope_parameters: Option<AhdsrParameters>,
    granular_parameters: Option<GranularParameters>,
    modulation_state: Option<SamplerModulationState>,
    active_parameters: Vec<Box<dyn Parameter>>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    transient: bool, // True if the generator can exhaust
    stopping: bool,  // True if stop has been called and we are waiting for voices to decay
    stopped: bool,   // True if all voices have decayed after a stop call
    options: GeneratorPlaybackOptions,
    output_sample_rate: u32,
    output_channel_count: usize,
    temp_buffer: Vec<f32>,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
    // Base sampler parameters (always active)
    pub const TRANSPOSE: IntegerParameter =
        IntegerParameter::new(FourCC(*b"STRN"), "Transpose", -48..=48, 0).with_unit("st");

    pub const FINETUNE: IntegerParameter =
        IntegerParameter::new(FourCC(*b"SFTN"), "Finetune", -100..=100, 0).with_unit("ct");

    pub const VOLUME: FloatParameter = FloatParameter::new(
        FourCC(*b"SVOL"),
        "Volume",
        0.000001..=15.848932, // db_to_linear(-60.0)..=db_to_linear(24.0)
        1.0,                  // 0dB
    );

    pub const PANNING: FloatParameter =
        FloatParameter::new(FourCC(*b"SPAN"), "Panning", -1.0..=1.0, 0.0);

    /// Base sampler parameter descriptors (transpose, finetune, volume, panning).
    pub fn base_parameters() -> Vec<Box<dyn Parameter>> {
        let gain_to_string = |v: f32| {
            let db = linear_to_db(v);
            if db <= -60.0 {
                "-INF".to_string()
            } else {
                format!("{:.2}", db)
            }
        };
        let string_to_gain = |s: &str| {
            if s.trim().eq_ignore_ascii_case("-inf") || s.trim().eq_ignore_ascii_case("inf") {
                Some(*Self::VOLUME.range().start())
            } else {
                let s = s.trim_start().trim_end_matches(|c: char| {
                    c.eq_ignore_ascii_case(&'d')
                        || c.eq_ignore_ascii_case(&'b')
                        || c.is_whitespace()
                });
                s.parse::<f32>().ok().map(db_to_linear)
            }
        };

        let pan_to_string = |v: f32| {
            let v = v * 50.0;
            if v.abs() < 0.1 {
                "C".to_string()
            } else if v < 0.0 {
                format!("{:.0}L", v.abs())
            } else {
                format!("{:.0}R", v)
            }
        };
        let string_to_pan = |s: &str| {
            let s = s.trim();
            if s.eq_ignore_ascii_case("c") {
                Some(0.0)
            } else {
                let last_char = s.trim_end().chars().last().unwrap_or(' ');
                if last_char.eq_ignore_ascii_case(&'l') {
                    let s = s.trim_start().trim_end_matches(|c: char| {
                        c.eq_ignore_ascii_case(&'l') || c.is_whitespace()
                    });
                    s.parse::<f32>().ok().map(|v| -v / 50.0)
                } else if last_char.eq_ignore_ascii_case(&'r') {
                    let s = s.trim_start().trim_end_matches(|c: char| {
                        c.eq_ignore_ascii_case(&'r') || c.is_whitespace()
                    });
                    s.parse::<f32>().ok().map(|v| v / 50.0)
                } else {
                    None
                }
            }
        };

        vec![
            Self::TRANSPOSE.into_box(),
            Self::FINETUNE.into_box(),
            Self::VOLUME
                .with_unit("dB")
                .with_scaling(ParameterScaling::Decibel(-60.0, 24.0))
                .with_display(gain_to_string, string_to_gain)
                .into_box(),
            Self::PANNING
                .with_display(pan_to_string, string_to_pan)
                .into_box(),
        ]
    }

    // Envelope parameters (only active when ahdsr playback is enabled)
    const MIN_TIME_SEC: f32 = 0.0;
    const MAX_TIME_SEC: f32 = 10.0;

    pub const AMP_ATTACK: FloatParameter = FloatParameter::new(
        FourCC(*b"AATK"),
        "Attack",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.001,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("s");
    pub const AMP_HOLD: FloatParameter = FloatParameter::new(
        FourCC(*b"AHLD"),
        "Hold",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.75,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("s");
    pub const AMP_DECAY: FloatParameter = FloatParameter::new(
        FourCC(*b"ADCY"),
        "Decay",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        0.5,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("s");
    pub const AMP_SUSTAIN: FloatParameter = FloatParameter::new(
        FourCC(*b"ASTN"), //
        "Sustain",
        0.0..=1.0,
        0.75,
    );
    pub const AMP_RELEASE: FloatParameter = FloatParameter::new(
        FourCC(*b"AREL"),
        "Release",
        Self::MIN_TIME_SEC..=Self::MAX_TIME_SEC,
        1.0,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("s");

    /// AHDSR envelope parameter descriptors.
    pub fn envelope_parameters() -> Vec<Box<dyn Parameter>> {
        vec![
            Self::AMP_ATTACK.into_box(),
            Self::AMP_HOLD.into_box(),
            Self::AMP_DECAY.into_box(),
            Self::AMP_SUSTAIN.into_box(),
            Self::AMP_RELEASE.into_box(),
        ]
    }

    /// Apply given [ParameterValueUpdate] to an [AhdsrParameters] object.
    pub fn set_envelope_parameter(
        params: &mut AhdsrParameters,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::AMP_ATTACK.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_ATTACK)?;
                params.set_attack_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_HOLD.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_HOLD)?;
                params.set_hold_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_DECAY.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_DECAY)?;
                params.set_decay_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ if id == Self::AMP_SUSTAIN.id() => {
                let sustain = Sampler::parameter_update_value(value, &Self::AMP_SUSTAIN)?;
                params.set_sustain_level(sustain)?;
            }
            _ if id == Self::AMP_RELEASE.id() => {
                let seconds = Sampler::parameter_update_value(value, &Self::AMP_RELEASE)?;
                params.set_release_time(Duration::from_secs_f32(seconds.max(0.0)))?;
            }
            _ => {
                return Err(Error::ParameterError(format!(
                    "Invalid/unknown envelope parameter '{id}'"
                )))
            }
        }
        Ok(())
    }

    // Granular playback parameters (only active when granular playback is enabled)
    const MIN_GRAIN_SIZE_MS: f32 = 1.0;
    const MAX_GRAIN_SIZE_MS: f32 = 1000.0;
    const MIN_GRAIN_DENSITY_HZ: f32 = 1.0;
    const MAX_GRAIN_DENSITY_HZ: f32 = 100.0;

    pub const GRAIN_OVERLAP_MODE: EnumParameter = EnumParameter::new(
        FourCC(*b"GOVM"),
        "Overlap Mode",
        GrainOverlapMode::VARIANTS,
        GrainOverlapMode::Cloud as usize,
    );

    pub const GRAIN_WINDOW: EnumParameter = EnumParameter::new(
        FourCC(*b"GWND"),
        "Window",
        GrainWindowMode::VARIANTS,
        GrainWindowMode::Hann as usize,
    );

    pub const GRAIN_SIZE: FloatParameter = FloatParameter::new(
        FourCC(*b"GSIZ"),
        "Grain Size",
        Self::MIN_GRAIN_SIZE_MS..=Self::MAX_GRAIN_SIZE_MS,
        100.0,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("ms");

    pub const GRAIN_DENSITY: FloatParameter = FloatParameter::new(
        FourCC(*b"GDEN"),
        "Density",
        Self::MIN_GRAIN_DENSITY_HZ..=Self::MAX_GRAIN_DENSITY_HZ,
        10.0,
    )
    .with_scaling(ParameterScaling::Exponential(2.0))
    .with_unit("Hz");

    pub const GRAIN_VARIATION: FloatParameter =
        FloatParameter::new(FourCC(*b"GVAR"), "Variation", 0.0..=1.0, 0.0);

    pub const GRAIN_SPRAY: FloatParameter =
        FloatParameter::new(FourCC(*b"GSPY"), "Spray", 0.0..=1.0, 0.0);

    pub const GRAIN_PAN_SPREAD: FloatParameter =
        FloatParameter::new(FourCC(*b"GPAN"), "Pan Spread", 0.0..=1.0, 0.0);

    pub const GRAIN_PLAYBACK_DIR: EnumParameter = EnumParameter::new(
        FourCC(*b"GDIR"),
        "Direction",
        GrainPlaybackDirection::VARIANTS,
        GrainPlaybackDirection::Forward as usize,
    );
    pub const GRAIN_POSITION: FloatParameter =
        FloatParameter::new(FourCC(*b"GPOS"), "Position", 0.0..=1.0, 0.5);

    pub const GRAIN_STEP: FloatParameter =
        FloatParameter::new(FourCC(*b"GSTP"), "Step", -4.0..=4.0, 0.0).with_unit("x");

    /// Granular playback parameter descriptors.
    pub fn granular_parameters() -> Vec<Box<dyn Parameter>> {
        let percent_to_string = |v: f32| format!("{:.1} %", v * 100.0);
        let string_to_percent = |s: &str| {
            let s = s
                .trim_start()
                .trim_end_matches(|c: char| c == '%' || c.is_whitespace());
            s.parse::<f32>().ok().map(|v| v / 100.0)
        };

        vec![
            Self::GRAIN_OVERLAP_MODE.into_box(),
            Self::GRAIN_WINDOW.into_box(),
            Self::GRAIN_SIZE.into_box(),
            Self::GRAIN_DENSITY.into_box(),
            Self::GRAIN_VARIATION
                .with_display(percent_to_string, string_to_percent)
                .into_box(),
            Self::GRAIN_SPRAY
                .with_display(percent_to_string, string_to_percent)
                .into_box(),
            Self::GRAIN_PAN_SPREAD
                .with_display(percent_to_string, string_to_percent)
                .into_box(),
            Self::GRAIN_PLAYBACK_DIR.into_box(),
            Self::GRAIN_POSITION
                .with_display(percent_to_string, string_to_percent)
                .into_box(),
            Self::GRAIN_STEP.into_box(),
        ]
    }

    /// Apply given [ParameterValueUpdate] to a [GranularParameters] object.
    pub fn set_granular_parameter(
        params: &mut GranularParameters,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::GRAIN_OVERLAP_MODE.id() => {
                let mut enum_value = EnumParameterValue::<GrainOverlapMode>::from_description(
                    Self::GRAIN_OVERLAP_MODE,
                );
                enum_value.apply_update(value);
                params.overlap_mode = enum_value.value();
            }
            _ if id == Self::GRAIN_WINDOW.id() => {
                let mut enum_value =
                    EnumParameterValue::<GrainWindowMode>::from_description(Self::GRAIN_WINDOW);
                enum_value.apply_update(value);
                params.window = enum_value.value();
            }
            _ if id == Self::GRAIN_SIZE.id() => {
                let ms = Sampler::parameter_update_value(value, &Self::GRAIN_SIZE)?;
                params.size = ms;
            }
            _ if id == Self::GRAIN_DENSITY.id() => {
                let hz = Sampler::parameter_update_value(value, &Self::GRAIN_DENSITY)?;
                params.density = hz;
            }
            _ if id == Self::GRAIN_VARIATION.id() => {
                let variation = Sampler::parameter_update_value(value, &Self::GRAIN_VARIATION)?;
                params.variation = variation;
            }
            _ if id == Self::GRAIN_SPRAY.id() => {
                let spray = Sampler::parameter_update_value(value, &Self::GRAIN_SPRAY)?;
                params.spray = spray;
            }
            _ if id == Self::GRAIN_PAN_SPREAD.id() => {
                let spread = Sampler::parameter_update_value(value, &Self::GRAIN_PAN_SPREAD)?;
                params.pan_spread = spread;
            }
            _ if id == Self::GRAIN_PLAYBACK_DIR.id() => {
                let mut enum_value = EnumParameterValue::<GrainPlaybackDirection>::from_description(
                    Self::GRAIN_PLAYBACK_DIR,
                );
                enum_value.apply_update(value);
                params.playback_direction = enum_value.value();
            }
            _ if id == Self::GRAIN_POSITION.id() => {
                let position = Sampler::parameter_update_value(value, &Self::GRAIN_POSITION)?;
                params.position = position;
            }
            _ if id == Self::GRAIN_STEP.id() => {
                let step = Sampler::parameter_update_value(value, &Self::GRAIN_STEP)?;
                params.step = step;
            }
            _ => {
                return Err(Error::ParameterError(format!(
                    "Invalid/unknown granular playback parameter '{id}'"
                )))
            }
        }
        Ok(())
    }

    // Modulation source descriptors
    pub const MOD_SOURCE_LFO1: FourCC = FourCC(*b"LFO1");
    pub const MOD_SOURCE_LFO2: FourCC = FourCC(*b"LFO2");
    pub const MOD_SOURCE_VELOCITY: FourCC = FourCC(*b"VELM");
    pub const MOD_SOURCE_KEYTRACK: FourCC = FourCC(*b"KEYM");

    // Modulation parameters - LFO 1
    pub const MOD_LFO1_RATE: FloatParameter =
        FloatParameter::new(FourCC(*b"ML1R"), "LFO 1 Rate", 0.01..=20.0, 1.0)
            .with_scaling(ParameterScaling::Exponential(2.0))
            .with_unit("Hz");
    pub const MOD_LFO1_WAVEFORM: EnumParameter = EnumParameter::new(
        FourCC(*b"ML1W"),
        "LFO 1 Waveform",
        LfoWaveform::VARIANTS,
        LfoWaveform::Sine as usize,
    );

    // Modulation parameters - LFO 2
    pub const MOD_LFO2_RATE: FloatParameter =
        FloatParameter::new(FourCC(*b"ML2R"), "LFO 2 Rate", 0.01..=20.0, 2.0)
            .with_scaling(ParameterScaling::Exponential(2.0))
            .with_unit("Hz");
    pub const MOD_LFO2_WAVEFORM: EnumParameter = EnumParameter::new(
        FourCC(*b"ML2W"),
        "LFO 2 Waveform",
        LfoWaveform::VARIANTS,
        LfoWaveform::Triangle as usize,
    );

    /// Modulation configuration for the sampler (with granular playback enabled).
    pub fn modulation_config() -> ModulationConfig {
        ModulationConfig {
            sources: vec![
                ModulationSource::Lfo {
                    id: Self::MOD_SOURCE_LFO1,
                    name: "LFO 1",
                    rate_param: Self::MOD_LFO1_RATE,
                    waveform_param: Self::MOD_LFO1_WAVEFORM,
                },
                ModulationSource::Lfo {
                    id: Self::MOD_SOURCE_LFO2,
                    name: "LFO 2",
                    rate_param: Self::MOD_LFO2_RATE,
                    waveform_param: Self::MOD_LFO2_WAVEFORM,
                },
                ModulationSource::Velocity {
                    id: Self::MOD_SOURCE_VELOCITY,
                    name: "Velocity",
                },
                ModulationSource::Keytracking {
                    id: Self::MOD_SOURCE_KEYTRACK,
                    name: "Keytracking",
                },
            ],
            targets: vec![
                ModulationTarget::new(Self::GRAIN_SIZE.id(), Self::GRAIN_SIZE.name()),
                ModulationTarget::new(Self::GRAIN_DENSITY.id(), Self::GRAIN_DENSITY.name()),
                ModulationTarget::new(Self::GRAIN_VARIATION.id(), Self::GRAIN_VARIATION.name()),
                ModulationTarget::new(Self::GRAIN_SPRAY.id(), Self::GRAIN_SPRAY.name()),
                ModulationTarget::new(Self::GRAIN_PAN_SPREAD.id(), Self::GRAIN_PAN_SPREAD.name()),
                ModulationTarget::new(Self::GRAIN_POSITION.id(), Self::GRAIN_POSITION.name()),
                ModulationTarget::new(Self::GRAIN_STEP.id(), Self::GRAIN_STEP.name()),
            ],
        }
    }

    /// Create a new sampler with the given sample file
    ///
    /// # Arguments
    /// * `file_path` - Full path to the sample file that should be played back.
    /// * `envelope_parameters` - Optional parameters for the volume AHDSR envelope.
    ///   When None, no envelope will be applied.
    /// * `options` - Generic generator playback options.
    /// * `output_sample_rate` - Output sample rate of the source -
    ///   usually the player's audio backend's sample rate.
    /// * `output_channel_count` - Output channel count -
    ///   usually the player's audio backend's channel count.
    pub fn from_file<P: AsRef<Path>>(
        file_path: P,
        options: GeneratorPlaybackOptions,
        output_channel_count: usize,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        let file_source = PreloadedFileSource::from_file(
            &file_path,
            FilePlaybackOptions::default(),
            output_sample_rate,
        )?;

        Self::from_file_source(
            file_source,
            options,
            output_channel_count,
            output_sample_rate,
        )
    }

    /// Create a new sampler with the given raw encoded sample file buffer.
    /// See [Self::from_file] for more info about the parameters.
    pub fn from_file_buffer<P: AsRef<Path>>(
        file_buffer: Vec<u8>,
        file_path: P,
        options: GeneratorPlaybackOptions,
        output_channel_count: usize,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        let file_path = file_path.as_ref().to_string_lossy().to_string();
        let file_source = PreloadedFileSource::from_file_buffer(
            file_buffer,
            &file_path,
            FilePlaybackOptions::default(),
            output_sample_rate,
        )?;

        Self::from_file_source(
            file_source,
            options,
            output_channel_count,
            output_sample_rate,
        )
    }

    /// Create a new sampler with the given preloaded file source.
    /// See [Self::from_file] for more info about the parameters.
    pub fn from_file_source(
        file_source: PreloadedFileSource,
        options: GeneratorPlaybackOptions,
        output_channel_count: usize,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // Memorize file path
        let file_path = Arc::new(file_source.file_name());

        // Pre-allocate playback message queue so it fits all parameters and a bunch of trigger events
        let playback_message_queue_size: usize = (Self::base_parameters().len()
            + Self::envelope_parameters().len()
            + Self::granular_parameters().len())
            * 2
            + 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(playback_message_queue_size));

        // Create a new unique source id
        let playback_id = unique_source_id();
        let playback_status_send = None;

        // Set voice playback options
        let mut voice_playback_options = FilePlaybackOptions::default();
        if let Some(duration) = options.playback_pos_emit_rate {
            voice_playback_options = voice_playback_options.playback_pos_emit_rate(duration);
        }
        // de-click, in case there's no envelope
        voice_playback_options.fade_out_duration = Some(Duration::from_millis(50));

        // Allocate voices
        let mut voices = Vec::with_capacity(options.voices);
        for _ in 0..options.voices {
            let file_source = file_source
                .clone(voice_playback_options, output_sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!("Failed to create sampler voice: {err}"))
                })?;
            voices.push(SamplerVoice::new(
                file_source,
                output_channel_count,
                output_sample_rate,
            ));
        }

        // Base parameter values
        let base_transpose = 0;
        let base_finetune = 0;
        let base_volume = 1.0;
        let base_panning = 0.0;

        // Optional parameters
        let envelope_parameters = None;
        let granular_parameters = None;

        // Modulation state (with empty config - will be initialized in with_granular_playback)
        let modulation_state = None;

        let active_voices = 0;

        // Base parameters are always active. Envelope and granular parameters are added when enabled.
        let active_parameters = Self::base_parameters();

        // Initial playback state
        let transient = false;
        let stopping = false;
        let stopped = false;

        // Pre-allocate temp buffer for mixing, using mixer's max sample buffer size
        let temp_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

        Ok(Self {
            playback_id,
            playback_message_queue,
            playback_status_send,
            file_path,
            active_voices,
            voices,
            base_transpose,
            base_finetune,
            base_volume,
            base_panning,
            envelope_parameters,
            granular_parameters,
            modulation_state,
            active_parameters,
            transient,
            stopping,
            stopped,
            options,
            output_sample_rate,
            output_channel_count,
            temp_buffer,
        })
    }

    /// Builder method to enable AHDSR envelope on the sampler.
    pub fn with_ahdsr(mut self, mut parameters: AhdsrParameters) -> Result<Self, Error> {
        // Initialize the parameters with the output sample rate
        parameters
            .set_sample_rate(self.output_sample_rate)
            .map_err(|err| {
                Error::ParameterError(format!("Failed to initialize AHDSR parameters: {err}"))
            })?;

        // Add AHDSR parameters to the active parameters list
        self.active_parameters.extend(Self::envelope_parameters());

        self.envelope_parameters = Some(parameters);
        Ok(self)
    }

    /// Builder method to enable granular playback on the sampler.
    pub fn with_granular_playback(mut self, parameters: GranularParameters) -> Result<Self, Error> {
        // Validate the parameters
        parameters
            .validate()
            .map_err(|err| Error::ParameterError(format!("Invalid granular parameters: {err}")))?;

        // Add granular parameters to the active parameters list
        self.active_parameters.extend(Self::granular_parameters());

        // Create modulation config
        let modulation_config = Self::modulation_config();

        // Add modulation parameters to the active parameters list
        self.active_parameters
            .extend(modulation_config.source_parameters());

        // Resample file source, if needed and mix down to mono
        let sample_buffer = Self::create_granular_sample_buffer(
            self.voices.first().unwrap().file_source().file_buffer(),
            self.output_sample_rate,
        )?;

        // Initialize modulation state
        let modulation_state = SamplerModulationState::new(modulation_config);

        // Initialize granular playback on all voices
        for voice in &mut self.voices {
            voice.enable_granular_playback(
                modulation_state.create_matrix(self.output_sample_rate),
                self.output_sample_rate,
                sample_buffer.clone(),
            );
        }

        self.modulation_state = Some(modulation_state);

        self.granular_parameters = Some(parameters);
        Ok(self)
    }

    /// Process pending playback messages from the queue.
    fn process_playback_messages(&mut self, current_sample_frame: u64) {
        while let Some(message) = self.playback_message_queue.pop() {
            match message {
                GeneratorPlaybackMessage::Stop => {
                    self.stop(current_sample_frame);
                }
                GeneratorPlaybackMessage::Trigger { event } => {
                    // Ignore all trigger messages while we're stopping
                    if !self.stopping {
                        match event {
                            GeneratorPlaybackEvent::AllNotesOff => {
                                self.trigger_all_notes_off(current_sample_frame);
                            }
                            GeneratorPlaybackEvent::NoteOn {
                                note_id,
                                note,
                                volume,
                                panning,
                                context,
                            } => {
                                self.trigger_note_on(note_id, note, volume, panning, context);
                            }
                            GeneratorPlaybackEvent::NoteOff { note_id } => {
                                self.trigger_note_off(note_id, current_sample_frame);
                            }
                            GeneratorPlaybackEvent::SetSpeed {
                                note_id,
                                speed,
                                glide,
                            } => {
                                self.trigger_set_speed(note_id, speed, glide);
                            }
                            GeneratorPlaybackEvent::SetVolume { note_id, volume } => {
                                self.trigger_set_volume(note_id, volume);
                            }
                            GeneratorPlaybackEvent::SetPanning { note_id, panning } => {
                                self.trigger_set_panning(note_id, panning);
                            }
                            GeneratorPlaybackEvent::SetParameter { id, value } => {
                                if let Err(err) = self.process_parameter_update(id, &value) {
                                    log::warn!("Failed to process parameter '{id}' update: {err}");
                                }
                            }
                            GeneratorPlaybackEvent::SetParameters { values } => {
                                if let Err(err) = self.process_parameter_updates(&values) {
                                    log::warn!("Failed to process parameter updates: {err}");
                                }
                            }
                            GeneratorPlaybackEvent::SetModulation {
                                source,
                                target,
                                amount,
                                bipolar,
                            } => {
                                if let Err(err) =
                                    self.set_modulation(source, target, amount, bipolar)
                                {
                                    log::warn!("Failed to set modulation: {err}");
                                }
                            }
                            GeneratorPlaybackEvent::ClearModulation { source, target } => {
                                if let Err(err) = self.clear_modulation(source, target) {
                                    log::warn!("Failed to clear modulation: {err}");
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn stop(&mut self, current_sample_frame: u64) {
        // Mark source as about to stop when this is a transient generator
        self.stopping = self.transient;
        // Stop all active voices, if any
        self.trigger_all_notes_off(current_sample_frame);
    }

    /// Immediately trigger a note on (used by event processor)
    fn trigger_note_on(
        &mut self,
        note_id: NotePlaybackId,
        note: u8,
        volume: Option<f32>,
        panning: Option<f32>,
        context: Option<PlaybackStatusContext>,
    ) {
        // Unwrap volume/pan
        let volume_value = volume.unwrap_or(1.0);
        let panning_value = panning.unwrap_or(0.0);

        // Allocate a new voice
        let voice_index = self.next_free_voice_index();
        let voice = &mut self.voices[voice_index];

        // Start the voice
        voice.start(
            note_id,
            note,
            volume_value,
            panning_value,
            self.base_transpose,
            self.base_finetune,
            self.base_volume,
            self.base_panning,
            &self.envelope_parameters,
            &self.granular_parameters,
            context,
        );

        // Ensure we're checking in the upcoming `write` if any voice needs processing.
        self.active_voices += 1;
    }

    fn trigger_note_off(&mut self, note_id: NotePlaybackId, current_sample_frame: u64) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.stop(&self.envelope_parameters, current_sample_frame);
            // NB: do not modify `active_voices` here. it's updated in `write`
        }
    }

    fn trigger_all_notes_off(&mut self, current_sample_frame: u64) {
        for voice in &mut self.voices {
            voice.stop(&self.envelope_parameters, current_sample_frame);
            // NB: do not modify `active_voices` here. it's updated in `write`
        }
    }

    fn trigger_set_speed(&mut self, note_id: NotePlaybackId, speed: f64, glide: Option<f32>) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_speed(speed, glide, self.base_transpose, self.base_finetune);
        }
    }

    fn trigger_set_volume(&mut self, note_id: NotePlaybackId, volume: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_volume(volume, self.base_volume);
        }
    }

    fn trigger_set_panning(&mut self, note_id: NotePlaybackId, panning: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_panning(panning, self.base_panning);
        }
    }

    /// Find a free voice or steal the oldest one.
    /// Returns the index of the new voice, which is always valid.
    fn next_free_voice_index(&self) -> usize {
        // Try to find a completely free voice first
        if let Some(index) = self.voices.iter().position(|v| !v.is_active()) {
            return index;
        }
        // If all voices are active, find the best candidate to steal
        // Prioritize:
        //   a) Longest releasing voice (earliest release_start_sample_frame)
        //   b) Oldest active voice (by playback_id)
        let mut candidate_index = 0;
        let mut earliest_release_time: Option<u64> = None;
        let mut oldest_active_playback_id: Option<NotePlaybackId> = None;
        for (index, voice) in self.voices.iter().enumerate() {
            if self.envelope_parameters.is_some() && voice.in_release_stage() {
                // This voice is in Release stage
                if let Some(release_time) = voice.release_start_frame() {
                    if earliest_release_time.is_none_or(|earliest| release_time < earliest) {
                        earliest_release_time = Some(release_time);
                        oldest_active_playback_id = None; // Reset active voices once we found a releasing voice
                        candidate_index = index;
                    }
                }
            } else if earliest_release_time.is_none() {
                // This voice is active (not in Release stage)
                // Only consider if we haven't found a releasing voice yet
                if let Some(playback_id) = voice.note_id() {
                    if oldest_active_playback_id.is_none_or(|oldest| playback_id < oldest) {
                        oldest_active_playback_id = Some(playback_id);
                        candidate_index = index;
                    }
                }
            }
        }
        candidate_index
    }

    fn parameter_update_value(
        value: &ParameterValueUpdate,
        descriptor: &FloatParameter,
    ) -> Result<f32, Error> {
        match value {
            ParameterValueUpdate::Normalized(norm) => {
                Ok(descriptor.denormalize_value(norm.clamp(0.0, 1.0)))
            }
            ParameterValueUpdate::Raw(raw) => {
                if let Some(v) = raw.downcast_ref::<f32>() {
                    Ok(descriptor.clamp_value(*v))
                } else if let Some(v) = raw.downcast_ref::<f64>() {
                    Ok(descriptor.clamp_value(*v as f32))
                } else {
                    Err(Error::ParameterError(format!(
                        "Unsupported payload type for sampler parameter '{}'",
                        descriptor.name()
                    )))
                }
            }
        }
    }

    fn parameter_update_value_integer(
        value: &ParameterValueUpdate,
        descriptor: &IntegerParameter,
    ) -> Result<i32, Error> {
        match value {
            ParameterValueUpdate::Normalized(norm) => {
                Ok(descriptor.denormalize_value(norm.clamp(0.0, 1.0)))
            }
            ParameterValueUpdate::Raw(raw) => {
                if let Some(v) = raw.downcast_ref::<i32>() {
                    Ok(descriptor.clamp_value(*v))
                } else if let Some(v) = raw.downcast_ref::<i64>() {
                    Ok(descriptor.clamp_value(*v as i32))
                } else {
                    Err(Error::ParameterError(format!(
                        "Unsupported payload type for sampler parameter '{}'",
                        descriptor.name()
                    )))
                }
            }
        }
    }

    fn create_granular_sample_buffer(
        file_buffer: Arc<PreloadedFileBuffer>,
        output_sample_rate: u32,
    ) -> Result<Arc<Box<[f32]>>, Error> {
        if file_buffer.channel_count() == 1 && file_buffer.sample_rate() == output_sample_rate {
            // No conversion necessary, just copy
            Ok(Arc::new(file_buffer.buffer().to_vec().into_boxed_slice()))
        } else {
            // Create a temporary source to perform resampling with disabled looping
            let mut source = PreloadedFileSource::from_shared_buffer(
                file_buffer.clone(),
                "granular temp sample",
                FilePlaybackOptions::default()
                    .playback_pos_emit_disabled()
                    .resampling_quality(ResamplingQuality::Default)
                    .repeat(0),
                output_sample_rate,
            )?;
            let mut dest_mono_buffer = Vec::with_capacity(
                (file_buffer.frame_count() as u64 * output_sample_rate as u64
                    / file_buffer.sample_rate() as u64) as usize
                    + 100,
            );
            let source_channel_count = source.channel_count();
            let mut temp_buffer = vec![0.0; 1024 * source_channel_count];
            let mut time = SourceTime::default();
            loop {
                // Read and resample, if needed
                let read = source.write(&mut temp_buffer, &time);
                if read == 0 {
                    break;
                }
                // Downmix to mono
                for frame in temp_buffer[..read].chunks(source_channel_count) {
                    dest_mono_buffer.push(frame.iter().sum::<f32>() / source_channel_count as f32);
                }
                time.add_frames(read as u64 / source_channel_count as u64);
            }
            // Ensure sample buffer is not empty
            if dest_mono_buffer.is_empty() {
                dest_mono_buffer.push(0.0);
            }
            Ok(Arc::new(dest_mono_buffer.into_boxed_slice()))
        }
    }
}

// -------------------------------------------------------------------------------------------------

impl Source for Sampler {
    fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn channel_count(&self) -> usize {
        self.output_channel_count
    }

    fn is_exhausted(&self) -> bool {
        self.stopped
    }

    fn weight(&self) -> usize {
        self.active_voices.max(1)
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // Process playback messages
        self.process_playback_messages(time.pos_in_frames);

        // Return empty handed when exhausted or when there are no active voices
        if self.stopped || (self.active_voices == 0 && !self.stopping) {
            return 0;
        }

        // Clear output
        clear_buffer(output);

        // Mix active voices into the output
        let mut active_voices = 0;
        assert!(self.temp_buffer.len() >= output.len());
        for voice in &mut self.voices {
            if voice.is_active() {
                let mix_buffer = &mut self.temp_buffer[..output.len()];
                clear_buffer(mix_buffer);
                let written = voice.process(
                    mix_buffer,
                    self.output_channel_count,
                    &self.envelope_parameters,
                    &self.granular_parameters,
                    time,
                );
                add_buffers(&mut output[..written], &mix_buffer[..written]);
                if voice.is_active() {
                    // count voices that are still active after processed
                    active_voices += 1;
                }
            }
        }

        // Update `active_voices` based on the actual state
        self.active_voices = active_voices;

        // Send a stop message when we got requested to stop and are now exhausted
        if self.stopping && active_voices == 0 {
            self.stopped = true;
            if let Some(sender) = &self.playback_status_send {
                if let Err(err) = sender.send(PlaybackStatusEvent::Stopped {
                    id: self.playback_id,
                    path: self.file_path.clone(),
                    context: None,
                    exhausted: true,
                }) {
                    log::warn!("Failed to send sampler playback status event: {err}");
                }
            }
        }

        // We've cleared the entire buffer, so return the entire buffer
        output.len()
    }
}

impl Generator for Sampler {
    fn generator_name(&self) -> String {
        self.file_path.to_string()
    }

    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_options(&self) -> &GeneratorPlaybackOptions {
        &self.options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        self.playback_message_queue.clone()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_send = sender.clone();
        for voice in &mut self.voices {
            voice.set_playback_status_sender(sender.clone());
        }
    }

    fn is_transient(&self) -> bool {
        self.transient
    }
    fn set_is_transient(&mut self, is_transient: bool) {
        self.transient = is_transient
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        self.active_parameters.iter().map(|p| p.as_ref()).collect()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        match id {
            // Base parameters
            _ if id == Sampler::TRANSPOSE.id() => {
                let semitones =
                    Sampler::parameter_update_value_integer(value, &Sampler::TRANSPOSE)?;
                self.base_transpose = semitones;
                // Recompute speed for all active voices
                for voice in &mut self.voices {
                    if voice.is_active() {
                        voice.set_base_pitch(self.base_transpose, self.base_finetune);
                    }
                }
                return Ok(());
            }
            _ if id == Sampler::FINETUNE.id() => {
                let cents = Sampler::parameter_update_value_integer(value, &Sampler::FINETUNE)?;
                self.base_finetune = cents;
                // Recompute speed for all active voices
                for voice in &mut self.voices {
                    if voice.is_active() {
                        voice.set_base_pitch(self.base_transpose, self.base_finetune);
                    }
                }
                return Ok(());
            }
            _ if id == Sampler::VOLUME.id() => {
                let volume = Sampler::parameter_update_value(value, &Sampler::VOLUME)?;
                self.base_volume = volume;
                // Recompute volume for all active voices
                for voice in &mut self.voices {
                    if voice.is_active() {
                        voice.set_base_volume(self.base_volume);
                    }
                }
                return Ok(());
            }
            _ if id == Sampler::PANNING.id() => {
                let panning = Sampler::parameter_update_value(value, &Sampler::PANNING)?;
                self.base_panning = panning;
                // Recompute panning for all active voices
                for voice in &mut self.voices {
                    if voice.is_active() {
                        voice.set_base_panning(self.base_panning);
                    }
                }
                return Ok(());
            }
            // Envelope parameters
            _ if id == Sampler::AMP_ATTACK.id()
                || id == Sampler::AMP_HOLD.id()
                || id == Sampler::AMP_DECAY.id()
                || id == Sampler::AMP_SUSTAIN.id()
                || id == Sampler::AMP_RELEASE.id() =>
            {
                if let Some(params) = &mut self.envelope_parameters {
                    return Self::set_envelope_parameter(params, id, value);
                }
            }
            // Granular parameters
            _ if id == Sampler::GRAIN_OVERLAP_MODE.id()
                || id == Sampler::GRAIN_WINDOW.id()
                || id == Sampler::GRAIN_SIZE.id()
                || id == Sampler::GRAIN_DENSITY.id()
                || id == Sampler::GRAIN_VARIATION.id()
                || id == Sampler::GRAIN_SPRAY.id()
                || id == Sampler::GRAIN_PAN_SPREAD.id()
                || id == Sampler::GRAIN_PLAYBACK_DIR.id()
                || id == Sampler::GRAIN_POSITION.id()
                || id == Sampler::GRAIN_STEP.id() =>
            {
                if let Some(params) = &mut self.granular_parameters {
                    return Self::set_granular_parameter(params, id, value);
                }
            }
            // Modulation Parameters
            _ if self
                .modulation_state
                .as_ref()
                .is_some_and(|state| state.is_source_parameter(id)) =>
            {
                let modulation_state = self.modulation_state.as_mut().unwrap();

                // Check if this is an LFO rate parameter
                let rate = if id == Self::MOD_LFO1_RATE.id() {
                    Some(Self::parameter_update_value(value, &Self::MOD_LFO1_RATE)?)
                } else if id == Self::MOD_LFO2_RATE.id() {
                    Some(Self::parameter_update_value(value, &Self::MOD_LFO2_RATE)?)
                } else {
                    None
                };
                // Check if this is an LFO waveform parameter
                let waveform = if id == Self::MOD_LFO1_WAVEFORM.id() {
                    let mut waveform_value =
                        EnumParameterValue::from_description(Self::MOD_LFO1_WAVEFORM);
                    waveform_value.apply_update(value);
                    Some(waveform_value.value())
                } else if id == Self::MOD_LFO2_WAVEFORM.id() {
                    let mut waveform_value =
                        EnumParameterValue::from_description(Self::MOD_LFO2_WAVEFORM);
                    waveform_value.apply_update(value);
                    Some(waveform_value.value())
                } else {
                    None
                };

                // Delegate to modulation state
                return modulation_state.apply_parameter_update(
                    id,
                    rate,
                    waveform,
                    &mut self.voices,
                );
            }
            _ => {}
        }
        Err(Error::ParameterError(format!(
            "Invalid or unknown sampler parameter: '{id}'"
        )))
    }

    fn modulation_sources(&self) -> Vec<ModulationSource> {
        if let Some(modulation_state) = &self.modulation_state {
            modulation_state.sources()
        } else {
            Vec::new()
        }
    }

    fn modulation_targets(&self) -> Vec<ModulationTarget> {
        if let Some(modulation_state) = &self.modulation_state {
            modulation_state.targets()
        } else {
            Vec::new()
        }
    }

    fn set_modulation(
        &mut self,
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), Error> {
        if let Some(modulation_state) = &self.modulation_state {
            for voice in &mut self.voices {
                if let Some(matrix) = voice.modulation_matrix_mut() {
                    modulation_state.set_modulation(matrix, source, target, amount, bipolar)?;
                }
            }
            Ok(())
        } else {
            Err(Error::ParameterError(
                "Modulation routing only available when granular playback is enabled".to_string(),
            ))
        }
    }

    fn clear_modulation(&mut self, source: FourCC, target: FourCC) -> Result<(), Error> {
        if let Some(modulation_state) = &self.modulation_state {
            for voice in &mut self.voices {
                if let Some(matrix) = voice.modulation_matrix_mut() {
                    modulation_state.clear_modulation(matrix, source, target)?;
                }
            }
            Ok(())
        } else {
            Err(Error::ParameterError(
                "Modulation routing only available when granular playback is enabled".to_string(),
            ))
        }
    }
}
