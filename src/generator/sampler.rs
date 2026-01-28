use std::{
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;
use strum::VariantNames;

use crate::{
    generator::{GeneratorPlaybackEvent, GeneratorPlaybackMessage},
    parameter::{
        EnumParameter, EnumParameterValue, FloatParameter, Parameter, ParameterValueUpdate,
    },
    source::{
        file::preloaded::PreloadedFileSource, mixed::MixedSource, unique_source_id, Source,
        SourceTime,
    },
    sources::PreloadedFileBuffer,
    utils::{
        ahdsr::AhdsrParameters,
        buffer::{add_buffers, clear_buffer},
        dsp::lfo::LfoWaveform,
    },
    Error, FilePlaybackOptions, FileSource, Generator, GeneratorPlaybackOptions, NotePlaybackId,
    ParameterScaling, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent, ResamplingQuality,
};

// -------------------------------------------------------------------------------------------------

mod granular;
mod voice;

use voice::SamplerVoice;

pub use granular::{
    GrainPlaybackDirection, GrainPlayheadMode, GrainWindowMode, GranularParameters,
};

// -------------------------------------------------------------------------------------------------

/// Basic sampler which plays a single audio file with optional AHDSR envelope and/or
/// granular playback on a limited set of voices.
///
/// AHDSR and granular parameters can be automated.
pub struct Sampler {
    playback_id: PlaybackId,
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    file_path: Arc<String>,
    voices: Vec<SamplerVoice>,
    active_voices: usize,
    envelope_parameters: Option<AhdsrParameters>,
    granular_parameters: Option<GranularParameters>,
    active_parameters: Vec<Box<dyn Parameter>>,
    playback_status_send: Option<SyncSender<PlaybackStatusEvent>>,
    transient: bool, // True if the generator can exhaust
    stopping: bool,  // True if stop has been called and we are waiting for voices to decay
    stopped: bool,   // True if all voices have decayed after a stop call
    options: GeneratorPlaybackOptions,
    output_sample_rate: u32,
    output_channel_count: usize,
    temp_buffer: Vec<f32>,
    // Modulation parameters
    mod_lfo1_rate: f32,
    mod_lfo1_waveform: EnumParameterValue<LfoWaveform>,
    mod_lfo1_to_grain_pos: f32,
    mod_lfo1_to_grain_size: f32,
    mod_lfo1_to_grain_density: f32,
    mod_lfo2_rate: f32,
    mod_lfo2_waveform: EnumParameterValue<LfoWaveform>,
    mod_lfo2_to_grain_pos: f32,
    mod_lfo2_to_grain_size: f32,
    mod_lfo2_to_grain_density: f32,
    mod_vel_to_grain_pos: f32,
    mod_vel_to_grain_size: f32,
    mod_vel_to_grain_density: f32,
    mod_key_to_grain_pos: f32,
    mod_key_to_grain_size: f32,
    mod_key_to_grain_density: f32,
}

// -------------------------------------------------------------------------------------------------

impl Sampler {
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

    // Amplitude envelope parameters
    pub const ENVELOPE_PARAMETERS: [&dyn Parameter; 5] = [
        &Self::AMP_ATTACK,
        &Self::AMP_HOLD,
        &Self::AMP_DECAY,
        &Self::AMP_SUSTAIN,
        &Self::AMP_RELEASE,
    ];

    // Granular playback parameters
    const MIN_GRAIN_SIZE_MS: f32 = 1.0;
    const MAX_GRAIN_SIZE_MS: f32 = 1000.0;
    const MIN_GRAIN_DENSITY_HZ: f32 = 1.0;
    const MAX_GRAIN_DENSITY_HZ: f32 = 100.0;

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
        10.0,
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
    pub const GRAIN_PLAYHEAD_MODE: EnumParameter = EnumParameter::new(
        FourCC(*b"GPHM"),
        "Playhead Mode",
        GrainPlayheadMode::VARIANTS,
        GrainPlayheadMode::Manual as usize,
    );
    pub const GRAIN_POSITION: FloatParameter =
        FloatParameter::new(FourCC(*b"GPOS"), "Position", 0.0..=1.0, 0.5);
    pub const GRAIN_SPEED: FloatParameter =
        FloatParameter::new(FourCC(*b"GSPD"), "Speed", 0.1..=4.0, 1.0);

    // Granular playback parameters
    pub const GRAIN_PARAMETERS: [&dyn Parameter; 10] = [
        &Self::GRAIN_WINDOW,
        &Self::GRAIN_SIZE,
        &Self::GRAIN_DENSITY,
        &Self::GRAIN_VARIATION,
        &Self::GRAIN_SPRAY,
        &Self::GRAIN_PAN_SPREAD,
        &Self::GRAIN_PLAYBACK_DIR,
        &Self::GRAIN_PLAYHEAD_MODE,
        &Self::GRAIN_POSITION,
        &Self::GRAIN_SPEED,
    ];

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
    pub const MOD_LFO1_TO_GRAIN_POS: FloatParameter =
        FloatParameter::new(FourCC(*b"M1GP"), "LFO 1 > Position", -1.0..=1.0, 0.0);
    pub const MOD_LFO1_TO_GRAIN_SIZE: FloatParameter =
        FloatParameter::new(FourCC(*b"M1GS"), "LFO 1 > Size", -1.0..=1.0, 0.0);
    pub const MOD_LFO1_TO_GRAIN_DENSITY: FloatParameter =
        FloatParameter::new(FourCC(*b"M1GD"), "LFO 1 > Density", -1.0..=1.0, 0.0);

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
    pub const MOD_LFO2_TO_GRAIN_POS: FloatParameter =
        FloatParameter::new(FourCC(*b"M2GP"), "LFO 2 > Position", -1.0..=1.0, 0.0);
    pub const MOD_LFO2_TO_GRAIN_SIZE: FloatParameter =
        FloatParameter::new(FourCC(*b"M2GS"), "LFO 2 > Size", -1.0..=1.0, 0.0);
    pub const MOD_LFO2_TO_GRAIN_DENSITY: FloatParameter =
        FloatParameter::new(FourCC(*b"M2GD"), "LFO 2 > Density", -1.0..=1.0, 0.0);

    // Modulation parameters - Velocity
    pub const MOD_VEL_TO_GRAIN_POS: FloatParameter =
        FloatParameter::new(FourCC(*b"VGPS"), "Velocity > Position", -1.0..=1.0, 0.0);
    pub const MOD_VEL_TO_GRAIN_SIZE: FloatParameter =
        FloatParameter::new(FourCC(*b"VGSZ"), "Velocity > Size", -1.0..=1.0, 0.0);
    pub const MOD_VEL_TO_GRAIN_DENSITY: FloatParameter =
        FloatParameter::new(FourCC(*b"VGDN"), "Velocity > Density", -1.0..=1.0, 0.0);

    // Modulation parameters - Keytracking
    pub const MOD_KEY_TO_GRAIN_POS: FloatParameter =
        FloatParameter::new(FourCC(*b"KGPS"), "Keytrack > Position", -1.0..=1.0, 0.0);
    pub const MOD_KEY_TO_GRAIN_SIZE: FloatParameter =
        FloatParameter::new(FourCC(*b"KGSZ"), "Keytrack > Size", -1.0..=1.0, 0.0);
    pub const MOD_KEY_TO_GRAIN_DENSITY: FloatParameter =
        FloatParameter::new(FourCC(*b"KGDN"), "Keytrack > Density", -1.0..=1.0, 0.0);

    // All modulation parameters
    pub const GRAIN_MODULATION_PARAMETERS: [&dyn Parameter; 16] = [
        // LFO 1
        &Self::MOD_LFO1_RATE,
        &Self::MOD_LFO1_WAVEFORM,
        &Self::MOD_LFO1_TO_GRAIN_POS,
        &Self::MOD_LFO1_TO_GRAIN_SIZE,
        &Self::MOD_LFO1_TO_GRAIN_DENSITY,
        // LFO 2
        &Self::MOD_LFO2_RATE,
        &Self::MOD_LFO2_WAVEFORM,
        &Self::MOD_LFO2_TO_GRAIN_POS,
        &Self::MOD_LFO2_TO_GRAIN_SIZE,
        &Self::MOD_LFO2_TO_GRAIN_DENSITY,
        // Velocity
        &Self::MOD_VEL_TO_GRAIN_POS,
        &Self::MOD_VEL_TO_GRAIN_SIZE,
        &Self::MOD_VEL_TO_GRAIN_DENSITY,
        // Keytracking
        &Self::MOD_KEY_TO_GRAIN_POS,
        &Self::MOD_KEY_TO_GRAIN_SIZE,
        &Self::MOD_KEY_TO_GRAIN_DENSITY,
    ];

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
        envelope_parameters: Option<AhdsrParameters>,
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
            envelope_parameters,
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
        envelope_parameters: Option<AhdsrParameters>,
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
            envelope_parameters,
            options,
            output_channel_count,
            output_sample_rate,
        )
    }

    /// Create a new sampler with the given preloaded file source.
    /// See [Self::from_file] for more info about the parameters.
    pub fn from_file_source(
        file_source: PreloadedFileSource,
        envelope_parameters: Option<AhdsrParameters>,
        options: GeneratorPlaybackOptions,
        output_channel_count: usize,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // Memorize file path
        let file_path = Arc::new(file_source.file_name());

        // Pre-allocate playback message queue
        const PLAYBACK_MESSAGE_QUEUE_SIZE: usize = 10 + 16;
        let playback_message_queue = Arc::new(ArrayQueue::new(PLAYBACK_MESSAGE_QUEUE_SIZE));

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

        // Initialize envelope parameters, if any
        let mut envelope_parameters = envelope_parameters;
        if let Some(envelope_parameters) = &mut envelope_parameters {
            envelope_parameters
                .set_sample_rate(output_sample_rate)
                .map_err(|err| {
                    Error::ParameterError(format!(
                        "Failed to create envelope parameters for sampler: {err}"
                    ))
                })?;
        }

        let granular_parameters = None;

        let active_voices = 0;

        // Collect active parameters
        let mut active_parameters = Vec::new();
        if envelope_parameters.is_some() {
            active_parameters.extend([
                Self::AMP_ATTACK.into_box(),
                Self::AMP_HOLD.into_box(),
                Self::AMP_DECAY.into_box(),
                Self::AMP_SUSTAIN.into_box(),
                Self::AMP_RELEASE.into_box(),
            ]);
        }

        // Initial playback state
        let transient = false;
        let stopping = false;
        let stopped = false;

        // Pre-allocate temp buffer for mixing, using mixer's max sample buffer size
        let temp_buffer = vec![0.0; MixedSource::MAX_MIX_BUFFER_SAMPLES];

        let mut mod_env1_parameters = AhdsrParameters::default();
        mod_env1_parameters.set_sample_rate(output_sample_rate)?;

        Ok(Self {
            playback_id,
            playback_message_queue,
            playback_status_send,
            file_path,
            voices,
            active_voices,
            envelope_parameters,
            granular_parameters,
            active_parameters,
            transient,
            stopping,
            stopped,
            options,
            output_sample_rate,
            output_channel_count,
            temp_buffer,
            mod_lfo1_rate: Self::MOD_LFO1_RATE.default_value(),
            mod_lfo1_waveform: EnumParameterValue::from_description(Self::MOD_LFO1_WAVEFORM),
            mod_lfo1_to_grain_pos: Self::MOD_LFO1_TO_GRAIN_POS.default_value(),
            mod_lfo1_to_grain_size: Self::MOD_LFO1_TO_GRAIN_SIZE.default_value(),
            mod_lfo1_to_grain_density: Self::MOD_LFO1_TO_GRAIN_DENSITY.default_value(),
            mod_lfo2_rate: Self::MOD_LFO2_RATE.default_value(),
            mod_lfo2_waveform: EnumParameterValue::from_description(Self::MOD_LFO2_WAVEFORM),
            mod_lfo2_to_grain_pos: Self::MOD_LFO2_TO_GRAIN_POS.default_value(),
            mod_lfo2_to_grain_size: Self::MOD_LFO2_TO_GRAIN_SIZE.default_value(),
            mod_lfo2_to_grain_density: Self::MOD_LFO2_TO_GRAIN_DENSITY.default_value(),
            mod_vel_to_grain_pos: Self::MOD_VEL_TO_GRAIN_POS.default_value(),
            mod_vel_to_grain_size: Self::MOD_VEL_TO_GRAIN_SIZE.default_value(),
            mod_vel_to_grain_density: Self::MOD_VEL_TO_GRAIN_DENSITY.default_value(),
            mod_key_to_grain_pos: Self::MOD_KEY_TO_GRAIN_POS.default_value(),
            mod_key_to_grain_size: Self::MOD_KEY_TO_GRAIN_SIZE.default_value(),
            mod_key_to_grain_density: Self::MOD_KEY_TO_GRAIN_DENSITY.default_value(),
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
        self.active_parameters
            .extend(Self::ENVELOPE_PARAMETERS.iter().map(|p| p.dyn_clone()));

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
        self.active_parameters
            .extend(Self::GRAIN_PARAMETERS.iter().map(|p| p.dyn_clone()));

        // Add modulation parameters to the active parameters list
        self.active_parameters.extend(
            Self::GRAIN_MODULATION_PARAMETERS
                .iter()
                .map(|p| p.dyn_clone()),
        );

        // Resample file source, if needed
        let sample_buffer = Self::create_granular_sample_buffer(
            self.voices.first().unwrap().file_source().file_buffer(),
            self.output_sample_rate,
        )?;

        // Initialize granular playback on all voices
        for voice in &mut self.voices {
            voice.enable_granular_playback(self.output_sample_rate, sample_buffer.clone());
        }

        // Pre-allocate modulation slots for all voices (avoid allocation in audio thread)
        self.preallocate_modulation_slots();

        self.granular_parameters = Some(parameters);
        Ok(self)
    }

    /// Apply given [ParameterValueUpdate] to an [AhdsrParameters] object.
    pub fn apply_envelope_parameter_update(
        id: FourCC,
        value: &ParameterValueUpdate,
        params: &mut AhdsrParameters,
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

    /// Apply given [ParameterValueUpdate] to a [GranularParameters] object.
    pub fn apply_granular_playback_parameter_update(
        id: FourCC,
        value: &ParameterValueUpdate,
        params: &mut GranularParameters,
    ) -> Result<(), Error> {
        match id {
            _ if id == Self::GRAIN_WINDOW.id() => {
                let mut enum_value =
                    EnumParameterValue::<GrainWindowMode>::from_description(Self::GRAIN_WINDOW);
                enum_value.apply_update(value);
                params.grain_window = enum_value.value();
            }
            _ if id == Self::GRAIN_SIZE.id() => {
                let ms = Sampler::parameter_update_value(value, &Self::GRAIN_SIZE)?;
                params.grain_size = ms;
            }
            _ if id == Self::GRAIN_DENSITY.id() => {
                let hz = Sampler::parameter_update_value(value, &Self::GRAIN_DENSITY)?;
                params.grain_density = hz;
            }
            _ if id == Self::GRAIN_VARIATION.id() => {
                let variation = Sampler::parameter_update_value(value, &Self::GRAIN_VARIATION)?;
                params.grain_variation = variation;
            }
            _ if id == Self::GRAIN_SPRAY.id() => {
                let spray = Sampler::parameter_update_value(value, &Self::GRAIN_SPRAY)?;
                params.grain_spray = spray;
            }
            _ if id == Self::GRAIN_PAN_SPREAD.id() => {
                let spread = Sampler::parameter_update_value(value, &Self::GRAIN_PAN_SPREAD)?;
                params.grain_pan_spread = spread;
            }
            _ if id == Self::GRAIN_PLAYBACK_DIR.id() => {
                let mut enum_value = EnumParameterValue::<GrainPlaybackDirection>::from_description(
                    Self::GRAIN_PLAYBACK_DIR,
                );
                enum_value.apply_update(value);
                params.playback_direction = enum_value.value();
            }
            _ if id == Self::GRAIN_PLAYHEAD_MODE.id() => {
                let mut enum_value = EnumParameterValue::<GrainPlayheadMode>::from_description(
                    Self::GRAIN_PLAYHEAD_MODE,
                );
                enum_value.apply_update(value);
                params.playhead_mode = enum_value.value();
            }
            _ if id == Self::GRAIN_POSITION.id() => {
                let position = Sampler::parameter_update_value(value, &Self::GRAIN_POSITION)?;
                params.manual_position = position;
            }
            _ if id == Self::GRAIN_SPEED.id() => {
                let speed = Sampler::parameter_update_value(value, &Self::GRAIN_SPEED)?;
                params.playhead_speed = speed;
            }
            _ => {
                return Err(Error::ParameterError(format!(
                    "Invalid/unknown granular playback parameter '{id}'"
                )))
            }
        }
        Ok(())
    }

    /// Pre-allocate modulation slots for all voices (called during initialization, not in audio thread).
    fn preallocate_modulation_slots(&mut self) {
        use crate::utils::dsp::modulation::{
            KeytrackingModulationSource, LfoModulationSource, ModulationSlot,
            VelocityModulationSource,
        };

        for voice in &mut self.voices {
            let modulation_matrix = &mut voice.modulation_matrix;

            // 2 LFO slots (even if not used, to avoid allocation later)
            while modulation_matrix.lfo_slots.len() < 2 {
                let source =
                    LfoModulationSource::new(self.output_sample_rate, 1.0, Default::default());
                let slot = ModulationSlot::new(source);
                modulation_matrix.add_lfo_slot(slot);
            }

            // velocity slot
            if modulation_matrix.velocity_slot.is_none() {
                let source = VelocityModulationSource::new(1.0);
                let slot = ModulationSlot::new(source);
                modulation_matrix.set_velocity_slot(slot);
            }

            // keytracking slot
            if modulation_matrix.keytracking_slot.is_none() {
                let source = KeytrackingModulationSource::new(60.0);
                let slot = ModulationSlot::new(source);
                modulation_matrix.set_keytracking_slot(slot);
            }
        }
    }

    /// Update a voice's modulation matrix with current settings (called on note-on, in audio thread).
    /// This only updates existing pre-allocated slots without allocating memory.
    fn update_modulation_matrix(&mut self, voice_index: usize, note: u8, velocity: f32) {
        let voice = &mut self.voices[voice_index];
        let modulation_matrix = &mut voice.modulation_matrix;

        // Update LFO 1 slot (index 0)
        if let Some(slot) = modulation_matrix.lfo_slots.get_mut(0) {
            slot.source.set_rate(self.mod_lfo1_rate as f64);
            slot.source.set_waveform(self.mod_lfo1_waveform.value());
            slot.clear_targets();
            slot.update_target(Self::GRAIN_POSITION.id(), self.mod_lfo1_to_grain_pos, true);
            slot.update_target(Self::GRAIN_SIZE.id(), self.mod_lfo1_to_grain_size, true);
            slot.update_target(
                Self::GRAIN_DENSITY.id(),
                self.mod_lfo1_to_grain_density,
                true,
            );
        }

        // Update LFO 2 slot (index 1)
        if let Some(slot) = modulation_matrix.lfo_slots.get_mut(1) {
            slot.source.set_rate(self.mod_lfo2_rate as f64);
            slot.source.set_waveform(self.mod_lfo2_waveform.value());
            slot.clear_targets();
            slot.update_target(Self::GRAIN_POSITION.id(), self.mod_lfo2_to_grain_pos, true);
            slot.update_target(Self::GRAIN_SIZE.id(), self.mod_lfo2_to_grain_size, true);
            slot.update_target(
                Self::GRAIN_DENSITY.id(),
                self.mod_lfo2_to_grain_density,
                true,
            );
        }

        // Update Velocity slot
        if let Some(ref mut slot) = modulation_matrix.velocity_slot {
            slot.source.set_velocity(velocity);
            slot.clear_targets();
            slot.update_target(Self::GRAIN_POSITION.id(), self.mod_vel_to_grain_pos, false);
            slot.update_target(Self::GRAIN_SIZE.id(), self.mod_vel_to_grain_size, false);
            slot.update_target(
                Self::GRAIN_DENSITY.id(),
                self.mod_vel_to_grain_density,
                false,
            );
        }

        // Update Keytracking slot
        if let Some(ref mut slot) = modulation_matrix.keytracking_slot {
            slot.source.set_midi_note(note as f32);
            slot.clear_targets();
            slot.update_target(Self::GRAIN_POSITION.id(), self.mod_key_to_grain_pos, false);
            slot.update_target(Self::GRAIN_SIZE.id(), self.mod_key_to_grain_size, false);
            slot.update_target(
                Self::GRAIN_DENSITY.id(),
                self.mod_key_to_grain_density,
                false,
            );
        }
    }

    /// Apply modulation parameter updates to the sampler.
    fn apply_modulation_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        // LFO 1 parameters
        if id == Self::MOD_LFO1_RATE.id() {
            self.mod_lfo1_rate = Self::parameter_update_value(value, &Self::MOD_LFO1_RATE)?;
            // Update all active voices' LFO 1 rate
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice
                        .modulation_matrix
                        .update_lfo_rate(0, self.mod_lfo1_rate as f64);
                }
            }
        } else if id == Self::MOD_LFO1_WAVEFORM.id() {
            self.mod_lfo1_waveform.apply_update(value);
            // Update all active voices' LFO 1 waveform
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice
                        .modulation_matrix
                        .update_lfo_waveform(0, self.mod_lfo1_waveform.value());
                }
            }
        } else if id == Self::MOD_LFO1_TO_GRAIN_POS.id() {
            self.mod_lfo1_to_grain_pos =
                Self::parameter_update_value(value, &Self::MOD_LFO1_TO_GRAIN_POS)?;
            // Update all active voices' LFO 1 → Grain Position target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        0,
                        Self::GRAIN_POSITION.id(),
                        self.mod_lfo1_to_grain_pos,
                        true,
                    );
                }
            }
        } else if id == Self::MOD_LFO1_TO_GRAIN_SIZE.id() {
            self.mod_lfo1_to_grain_size =
                Self::parameter_update_value(value, &Self::MOD_LFO1_TO_GRAIN_SIZE)?;
            // Update all active voices' LFO 1 → Grain Size target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        0,
                        Self::GRAIN_SIZE.id(),
                        self.mod_lfo1_to_grain_size,
                        true,
                    );
                }
            }
        } else if id == Self::MOD_LFO1_TO_GRAIN_DENSITY.id() {
            self.mod_lfo1_to_grain_density =
                Self::parameter_update_value(value, &Self::MOD_LFO1_TO_GRAIN_DENSITY)?;
            // Update all active voices' LFO 1 → Grain Density target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        0,
                        Self::GRAIN_DENSITY.id(),
                        self.mod_lfo1_to_grain_density,
                        true,
                    );
                }
            }
        }
        // LFO 2 parameters
        else if id == Self::MOD_LFO2_RATE.id() {
            self.mod_lfo2_rate = Self::parameter_update_value(value, &Self::MOD_LFO2_RATE)?;
            // Update all active voices' LFO 2 rate
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice
                        .modulation_matrix
                        .update_lfo_rate(1, self.mod_lfo2_rate as f64);
                }
            }
        } else if id == Self::MOD_LFO2_WAVEFORM.id() {
            self.mod_lfo2_waveform.apply_update(value);
            // Update all active voices' LFO 2 waveform
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice
                        .modulation_matrix
                        .update_lfo_waveform(1, self.mod_lfo2_waveform.value());
                }
            }
        } else if id == Self::MOD_LFO2_TO_GRAIN_POS.id() {
            self.mod_lfo2_to_grain_pos =
                Self::parameter_update_value(value, &Self::MOD_LFO2_TO_GRAIN_POS)?;
            // Update all active voices' LFO 2 → Grain Position target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        1,
                        Self::GRAIN_POSITION.id(),
                        self.mod_lfo2_to_grain_pos,
                        true,
                    );
                }
            }
        } else if id == Self::MOD_LFO2_TO_GRAIN_SIZE.id() {
            self.mod_lfo2_to_grain_size =
                Self::parameter_update_value(value, &Self::MOD_LFO2_TO_GRAIN_SIZE)?;
            // Update all active voices' LFO 2 → Grain Size target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        1,
                        Self::GRAIN_SIZE.id(),
                        self.mod_lfo2_to_grain_size,
                        true,
                    );
                }
            }
        } else if id == Self::MOD_LFO2_TO_GRAIN_DENSITY.id() {
            self.mod_lfo2_to_grain_density =
                Self::parameter_update_value(value, &Self::MOD_LFO2_TO_GRAIN_DENSITY)?;
            // Update all active voices' LFO 2 → Grain Density target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_lfo_target(
                        1,
                        Self::GRAIN_DENSITY.id(),
                        self.mod_lfo2_to_grain_density,
                        true,
                    );
                }
            }
        }
        // Velocity parameters
        else if id == Self::MOD_VEL_TO_GRAIN_POS.id() {
            self.mod_vel_to_grain_pos =
                Self::parameter_update_value(value, &Self::MOD_VEL_TO_GRAIN_POS)?;
            // Update all active voices' Velocity → Grain Position target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_velocity_target(
                        Self::GRAIN_POSITION.id(),
                        self.mod_vel_to_grain_pos,
                        false, // unipolar
                    );
                }
            }
        } else if id == Self::MOD_VEL_TO_GRAIN_SIZE.id() {
            self.mod_vel_to_grain_size =
                Self::parameter_update_value(value, &Self::MOD_VEL_TO_GRAIN_SIZE)?;
            // Update all active voices' Velocity → Grain Size target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_velocity_target(
                        Self::GRAIN_SIZE.id(),
                        self.mod_vel_to_grain_size,
                        false, // unipolar
                    );
                }
            }
        } else if id == Self::MOD_VEL_TO_GRAIN_DENSITY.id() {
            self.mod_vel_to_grain_density =
                Self::parameter_update_value(value, &Self::MOD_VEL_TO_GRAIN_DENSITY)?;
            // Update all active voices' Velocity → Grain Density target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_velocity_target(
                        Self::GRAIN_DENSITY.id(),
                        self.mod_vel_to_grain_density,
                        false, // unipolar
                    );
                }
            }
        }
        // Keytracking parameters
        else if id == Self::MOD_KEY_TO_GRAIN_POS.id() {
            self.mod_key_to_grain_pos =
                Self::parameter_update_value(value, &Self::MOD_KEY_TO_GRAIN_POS)?;
            // Update all active voices' Keytracking → Grain Position target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_keytracking_target(
                        Self::GRAIN_POSITION.id(),
                        self.mod_key_to_grain_pos,
                        false, // unipolar
                    );
                }
            }
        } else if id == Self::MOD_KEY_TO_GRAIN_SIZE.id() {
            self.mod_key_to_grain_size =
                Self::parameter_update_value(value, &Self::MOD_KEY_TO_GRAIN_SIZE)?;
            // Update all active voices' Keytracking → Grain Size target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_keytracking_target(
                        Self::GRAIN_SIZE.id(),
                        self.mod_key_to_grain_size,
                        false, // unipolar
                    );
                }
            }
        } else if id == Self::MOD_KEY_TO_GRAIN_DENSITY.id() {
            self.mod_key_to_grain_density =
                Self::parameter_update_value(value, &Self::MOD_KEY_TO_GRAIN_DENSITY)?;
            // Update all active voices' Keytracking → Grain Density target
            for voice in &mut self.voices {
                if voice.is_active() {
                    voice.modulation_matrix.update_keytracking_target(
                        Self::GRAIN_DENSITY.id(),
                        self.mod_key_to_grain_density,
                        false, // unipolar
                    );
                }
            }
        } else {
            return Err(Error::ParameterError(format!(
                "Invalid/unknown modulation parameter {id}"
            )));
        }
        Ok(())
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
        // Allocate a new voice
        let voice_index = self.next_free_voice_index();
        let volume_value = volume.unwrap_or(1.0);

        // Update modulation matrix for the newly triggered voice
        self.update_modulation_matrix(voice_index, note, volume_value);

        // Start the voice
        let voice = &mut self.voices[voice_index];
        voice.start(
            note_id,
            note,
            volume_value,
            panning.unwrap_or(0.0),
            &self.envelope_parameters,
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
            voice.set_speed(speed, glide);
        }
    }

    fn trigger_set_volume(&mut self, note_id: NotePlaybackId, volume: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_volume(volume);
        }
    }

    fn trigger_set_panning(&mut self, note_id: NotePlaybackId, panning: f32) {
        if let Some(voice) = self
            .voices
            .iter_mut()
            .find(|v| v.note_id() == Some(note_id))
        {
            voice.set_panning(panning);
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
        self.active_parameters
            .iter()
            .map(|p| p.as_ref() as &dyn Parameter)
            .collect()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        // Handle AHDSR parameters
        if let Some(params) = &mut self.envelope_parameters {
            if Self::ENVELOPE_PARAMETERS.iter().any(|p| p.id() == id) {
                Self::apply_envelope_parameter_update(id, value, params)?;
                return Ok(());
            }
        }
        // Handle granular parameters
        if let Some(params) = &mut self.granular_parameters {
            if Self::GRAIN_PARAMETERS.iter().any(|p| p.id() == id) {
                Self::apply_granular_playback_parameter_update(id, value, params)?;
                return Ok(());
            }
        }
        // Handle modulation parameters
        if Self::GRAIN_MODULATION_PARAMETERS
            .iter()
            .any(|p| p.id() == id)
        {
            self.apply_modulation_parameter_update(id, value)?;
            return Ok(());
        }
        // If we get here, the parameter wasn't handled and thus is unknown
        Err(Error::ParameterError(format!(
            "Unknown sampler parameter: {id}"
        )))
    }
}
