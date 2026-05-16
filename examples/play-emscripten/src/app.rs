use std::{cell::RefCell, collections::HashMap};

use serde::Serialize;

use phonic::{
    effects,
    four_cc::FourCC,
    generators::{
        FunDspGenerator, Metronome, ModulationConfig, ModulationSource, ModulationTarget, Sampler,
    },
    utils::db_to_linear,
    DefaultOutputDevice, Effect, EffectHandle, EffectId, Error, Generator, GeneratorPlaybackHandle,
    GeneratorPlaybackOptions, MixerHandle, NotePlaybackId, Parameter, ParameterPolarity,
    ParameterType, ParameterValueUpdate, Player, SequencerHandle,
};

// -------------------------------------------------------------------------------------------------

// Serializable parameter metadata for JavaScript

#[derive(Serialize)]
#[serde(tag = "type")]
pub enum ParameterTypeInfo {
    Float { step: f32 },
    Integer { step: f32 },
    Enum { values: Vec<String> },
    Boolean,
}

#[derive(Serialize)]
pub struct ParameterInfo {
    id: u32,
    name: String,
    #[serde(flatten)]
    parameter_type: ParameterTypeInfo,
    default: f32,
}

impl From<&dyn Parameter> for ParameterInfo {
    fn from(value: &dyn Parameter) -> Self {
        let id: u32 = value.id().into();
        let name = value.name().to_string();
        let parameter_type = match value.parameter_type() {
            ParameterType::Float { step, polarity: _ } => ParameterTypeInfo::Float { step },
            ParameterType::Integer { step, polarity: _ } => ParameterTypeInfo::Integer { step },
            ParameterType::Enum { values } => ParameterTypeInfo::Enum { values },
            ParameterType::Boolean => ParameterTypeInfo::Boolean,
        };
        let default = value.default_value();
        Self {
            id,
            name,
            parameter_type,
            default,
        }
    }
}

#[derive(Serialize)]
pub struct EffectInfo {
    name: String,
    parameters: Vec<ParameterInfo>,
}

impl From<&dyn Effect> for EffectInfo {
    fn from(effect: &dyn Effect) -> Self {
        let name = effect.name().to_string();
        let parameters = effect
            .parameters()
            .iter()
            .map(|p| ParameterInfo::from(*p))
            .collect();
        EffectInfo { name, parameters }
    }
}

#[derive(Serialize)]
pub struct SynthInfo {
    name: String,
    parameters: Vec<ParameterInfo>,
}

impl SynthInfo {
    fn new(name: &str, params: &[&dyn Parameter]) -> Self {
        let name = name.to_string();
        let parameters = params.iter().map(|p| ParameterInfo::from(*p)).collect();
        Self { name, parameters }
    }
}

#[derive(Serialize)]
pub struct ParamUpdate {
    id: u32,
    value: f32,
}

#[derive(Serialize)]
pub struct ModulationSourceInfo {
    id: u32,
    name: String,
    polarity: String,
    parameters: Vec<ParameterInfo>,
}

#[derive(Serialize)]
pub struct ModulationTargetInfo {
    id: u32,
    name: String,
}

#[derive(Serialize)]
pub struct ModulationRoutingUpdate {
    source_id: u32,
    target_id: u32,
    amount: f32,
    bipolar: bool,
}

// -------------------------------------------------------------------------------------------------

// FunDSP synth impls
#[path = "../../common/synths/mod.rs"]
mod synths;

#[derive(Clone, Copy, PartialEq)]
enum SynthType {
    Sub3,
    Organ,
    Dx7,
}

impl SynthType {
    fn info(&self) -> SynthInfo {
        match self {
            SynthType::Sub3 => SynthInfo::new("sub3", synths::sub3::parameters()),
            SynthType::Dx7 => SynthInfo::new("dx7", synths::dx7::parameters()),
            SynthType::Organ => SynthInfo::new("organ", synths::organ::parameters()),
        }
    }

    fn create_generator(
        &self,
        sample_rate: u32,
        voice_count: usize,
    ) -> Result<FunDspGenerator, Error> {
        let options = GeneratorPlaybackOptions::default()
            .voices(voice_count)
            .volume_db(-3.0);

        match self {
            Self::Sub3 => FunDspGenerator::with_parameters(
                "sub3_synth",
                synths::sub3::parameters(),
                None,
                synths::sub3::modulation_config(),
                synths::sub3::voice_factory,
                options,
                sample_rate,
            ),
            Self::Organ => FunDspGenerator::with_parameters(
                "organ_synth",
                synths::organ::parameters(),
                None,
                synths::organ::modulation_config(),
                synths::organ::voice_factory,
                options,
                sample_rate,
            ),
            Self::Dx7 => FunDspGenerator::with_parameters(
                "dx7_synth",
                synths::dx7::parameters(),
                None,
                ModulationConfig::default(), // no modulation
                synths::dx7::voice_factory,
                options,
                sample_rate,
            ),
        }
    }

    fn modulation_config(&self) -> Option<ModulationConfig> {
        match self {
            Self::Sub3 => Some(synths::sub3::modulation_config()),
            Self::Organ => Some(synths::organ::modulation_config()),
            Self::Dx7 => None,
        }
    }

    fn randomize_modulation(&self) -> Vec<(FourCC, FourCC, f32, bool)> {
        match self {
            Self::Sub3 => synths::sub3::randomize_modulation(),
            Self::Organ => synths::organ::randomize_modulation(),
            Self::Dx7 => Vec::new(),
        }
    }

    fn parameters(&self) -> &[&dyn Parameter] {
        match self {
            SynthType::Sub3 => synths::sub3::parameters(),
            SynthType::Dx7 => synths::dx7::parameters(),
            SynthType::Organ => synths::organ::parameters(),
        }
    }

    fn randomize(&self) -> Vec<(FourCC, f32)> {
        match self {
            Self::Sub3 => synths::sub3::randomize(),
            Self::Organ => synths::organ::randomize(),
            Self::Dx7 => synths::dx7::randomize(),
        }
    }
}

// -------------------------------------------------------------------------------------------------

// Example Application

// Hold the data structures statically so we can bind the Emscripten C method callbacks.
// Use thread_local! instead of a LazyCell to avoid using mutexes here:
// We're getting called from a single thread in the browser anyway.
thread_local!(pub static APP: RefCell<Option<App>> = const { RefCell::new(None) });

pub struct App {
    player: Player,
    metronome_enabled: bool,
    metronome_generator: GeneratorPlaybackHandle,
    metronome_sequencer: Option<SequencerHandle>,
    voice_count: usize,
    active_synth: SynthType,
    synth_handle: GeneratorPlaybackHandle,
    synth_modulation_sources: Vec<ModulationSource>,
    synth_modulation_targets: Vec<ModulationTarget>,
    playing_notes: HashMap<u8, NotePlaybackId>,
    synth_mixer: MixerHandle,
    active_effects: HashMap<EffectId, (EffectHandle, Vec<Box<dyn Parameter>>)>,
}

impl App {
    // Create a new player, preload samples and create synths
    pub fn new() -> Result<Self, Error> {
        println!("Initialize audio output...");
        let output = DefaultOutputDevice::open()?;

        println!("Creating audio player...");
        let mut player = Player::new(output, None);

        // lower master volume a bit
        player.set_output_volume(db_to_linear(-3.0));

        let metronome_enabled = true;

        println!("Creating synths...");
        // create a new mixer for the synth
        let synth_mixer = player.add_mixer(None)?;
        // create and add the initial synth (Sub3)
        let voice_count = 8;
        let active_synth = SynthType::Sub3;
        let synth_generator =
            active_synth.create_generator(player.output_sample_rate(), voice_count)?;
        let synth_modulation_sources = synth_generator.modulation_sources();
        let synth_modulation_targets = synth_generator.modulation_targets();
        let synth_handle = player.add_generator(synth_generator, synth_mixer.id())?;
        let active_effects = HashMap::new();

        println!("Creating metronome...");
        let metronome_generator = player.add_generator(
            Sampler::from_file(
                "./assets/cowbell.wav",
                GeneratorPlaybackOptions::default().voices(2),
                player.output_channel_count(),
                player.output_sample_rate(),
            )?,
            None,
        )?;
        let start_time =
            player.output_sample_frame_position() + player.transport().seconds_to_samples(1.0);
        let metronome_sequencer = if metronome_enabled {
            Some(player.play_sequencer(
                Metronome::new(usize::MAX),
                metronome_generator.clone(),
                start_time,
            )?)
        } else {
            None
        };

        let playing_notes = HashMap::new();

        Ok(Self {
            player,
            metronome_enabled,
            metronome_generator,
            metronome_sequencer,
            voice_count,
            synth_mixer,
            active_synth,
            synth_handle,
            synth_modulation_sources,
            synth_modulation_targets,
            playing_notes,
            active_effects,
        })
    }

    // Player's current average CPU load
    pub fn cpu_load(&self) -> f32 {
        self.player.cpu_load().unwrap_or_default().average
    }

    // Set the active synth type
    pub fn set_active_synth(&mut self, synth_type: i32) -> Result<(), Error> {
        let new_synth_type = match synth_type {
            0 => SynthType::Sub3,
            1 => SynthType::Organ,
            2 | _ => SynthType::Dx7,
        };
        if new_synth_type != self.active_synth {
            self.active_synth = new_synth_type;
            // Stop all playing notes
            self.playing_notes.clear();
            // Remove the old synth
            self.player.remove_generator(self.synth_handle.id())?;
            // Create and add the new synth
            let synth_generator = new_synth_type
                .create_generator(self.player.output_sample_rate(), self.voice_count)?;
            self.synth_modulation_sources = synth_generator.modulation_sources();
            self.synth_modulation_targets = synth_generator.modulation_targets();
            self.synth_handle = self
                .player
                .add_generator(synth_generator, self.synth_mixer.id())?;
        }
        Ok(())
    }

    // Set the voice count for the active synth
    pub fn set_synth_voice_count(&mut self, voice_count: usize) -> Result<(), Error> {
        if self.voice_count != voice_count {
            self.voice_count = voice_count;
            // Stop all playing notes
            self.playing_notes.clear();
            // Remove old generator
            self.player.remove_generator(self.synth_handle.id())?;
            // Recreate the active synth with the new voice count
            let synth_generator = self
                .active_synth
                .create_generator(self.player.output_sample_rate(), voice_count)?;
            // Extract modulation info before adding to player
            self.synth_modulation_sources = synth_generator.modulation_sources();
            self.synth_modulation_targets = synth_generator.modulation_targets();
            self.synth_handle = self
                .player
                .add_generator(synth_generator, self.synth_mixer.id())?;
        }
        Ok(())
    }

    // Set metronome enabled state
    pub fn set_metronome_enabled(&mut self, enabled: bool) {
        if self.metronome_enabled == enabled {
            return;
        }
        self.metronome_enabled = enabled;
        if enabled {
            if let Ok(handle) = self.player.play_sequencer(
                Metronome::new(usize::MAX),
                self.metronome_generator.clone(),
                None,
            ) {
                self.metronome_sequencer = Some(handle);
            }
        } else if let Some(handle) = self.metronome_sequencer.take() {
            let _ = handle.stop(None);
        }
    }

    // Get parameters for the active synth
    pub fn get_active_synth_parameters(&self) -> SynthInfo {
        self.active_synth.info()
    }

    // Convert a synth parameter's value to a string
    pub fn synth_parameter_value_to_string(
        &self,
        param_id: FourCC,
        normalized_value: f32,
    ) -> Result<String, Error> {
        if let Some(parameter) = self
            .active_synth
            .parameters()
            .iter()
            .find(|p| p.id() == param_id)
        {
            return Ok(parameter.value_to_string(normalized_value, true));
        }
        if let Some(config) = self.active_synth.modulation_config() {
            if let Some(parameter) = config
                .source_parameters()
                .iter()
                .find(|p| p.id() == param_id)
            {
                return Ok(parameter.value_to_string(normalized_value, true));
            }
        }
        Err(Error::ParameterError(format!(
            "Parameter {param_id} not found in active synth",
        )))
    }

    // Convert a synth parameter's string to a value
    pub fn synth_parameter_string_to_value(
        &self,
        param_id: FourCC,
        string: String,
    ) -> Result<Option<f32>, Error> {
        if let Some(parameter) = self
            .active_synth
            .parameters()
            .iter()
            .find(|p| p.id() == param_id)
        {
            return Ok(parameter.string_to_value(string));
        }
        if let Some(config) = self.active_synth.modulation_config() {
            if let Some(parameter) = config
                .source_parameters()
                .iter()
                .find(|p| p.id() == param_id)
            {
                return Ok(parameter.string_to_value(string));
            }
        }
        Err(Error::ParameterError(format!(
            "Parameter {param_id} not found in active synth",
        )))
    }

    // Set a parameter value for the active synth
    pub fn set_synth_parameter_value(&mut self, param_id: FourCC, normalized_value: f32) {
        use ParameterValueUpdate::Normalized;
        let _ = self.synth_handle.set_parameter(
            (param_id, Normalized(normalized_value.clamp(0.0, 1.0))),
            None,
        );
    }

    // Trigger a synth note on.
    pub fn synth_note_on(&mut self, note: u8, velocity: f32) {
        // If a note is already playing for this key, stop it first.
        if let Some(note_id) = self.playing_notes.remove(&note) {
            let _ = self.synth_handle.note_off(note_id, None);
        }

        // Trigger the new note with the specified velocity and default panning
        if let Ok(note_id) = self.synth_handle.note_on(note, Some(velocity), None, None) {
            self.playing_notes.insert(note, note_id);
        }
    }

    // Trigger a synth note off.
    pub fn synth_note_off(&mut self, note: u8) {
        if let Some(note_id) = self.playing_notes.remove(&note) {
            let _ = self.synth_handle.note_off(note_id, None);
        }
    }

    // Randomize synth parameters and modulation
    pub fn randomize_synth(&self) -> (Vec<ParamUpdate>, Vec<ModulationRoutingUpdate>) {
        let mut param_updates = Vec::new();
        for (id, value) in self.active_synth.randomize() {
            use ParameterValueUpdate::Normalized;
            if self
                .synth_handle
                .set_parameter((id, Normalized(value)), None)
                .is_ok()
            {
                param_updates.push(ParamUpdate {
                    id: id.into(),
                    value,
                });
            }
        }

        let mut mod_updates = Vec::new();
        for (source, target, amount, bipolar) in self.active_synth.randomize_modulation() {
            if self
                .synth_handle
                .set_modulation(source, target, amount, bipolar, None)
                .is_ok()
            {
                mod_updates.push(ModulationRoutingUpdate {
                    source_id: source.into(),
                    target_id: target.into(),
                    amount,
                    bipolar,
                });
            }
        }

        (param_updates, mod_updates)
    }

    // Get modulation sources for the active synth
    pub fn get_modulation_sources(&self) -> Vec<ModulationSourceInfo> {
        let sources: Vec<ModulationSourceInfo> = self
            .synth_modulation_sources
            .iter()
            .map(|source| {
                let params: Vec<ParameterInfo> = source
                    .parameters()
                    .iter()
                    .map(|&p| ParameterInfo::from(p))
                    .collect();
                ModulationSourceInfo {
                    id: source.id().into(),
                    name: source.name().to_string(),
                    polarity: match source.polarity() {
                        ParameterPolarity::Unipolar => "unipolar".to_string(),
                        ParameterPolarity::Bipolar => "bipolar".to_string(),
                    },
                    parameters: params,
                }
            })
            .collect();
        sources
    }

    // Get modulation targets for the active synth
    pub fn get_modulation_targets(&self) -> Vec<ModulationTargetInfo> {
        let targets: Vec<ModulationTargetInfo> = self
            .synth_modulation_targets
            .iter()
            .map(|target| ModulationTargetInfo {
                id: target.id().into(),
                name: target.name().into(),
            })
            .collect();
        targets
    }

    // Set modulation routing
    pub fn set_modulation(
        &mut self,
        source_id: FourCC,
        target_id: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), Error> {
        self.synth_handle
            .set_modulation(source_id, target_id, amount, bipolar, None)
    }

    // Clear modulation routing
    pub fn clear_modulation(&mut self, source_id: FourCC, target_id: FourCC) -> Result<(), Error> {
        self.synth_handle
            .clear_modulation(source_id, target_id, None)
    }

    // Get list of available effects
    pub fn get_available_effects() -> Vec<String> {
        vec![
            "Gain".to_string(),
            "Panning".to_string(),
            "Filter".to_string(),
            "Eq5".to_string(),
            "Delay".to_string(),
            "Reverb".to_string(),
            "Chorus".to_string(),
            "Compressor".to_string(),
            "Gate".to_string(),
            "Distortion".to_string(),
        ]
    }

    // Add an effect by name to the synth mixer, returning effect ID and parameter metadata JSON
    pub fn add_effect_with_name(&mut self, effect_name: &str) -> Result<(EffectId, String), Error> {
        match effect_name {
            "Gain" => self.add_effect(effects::GainEffect::new()),
            "Panning" => self.add_effect(effects::PanningEffect::new()),
            "Filter" => self.add_effect(effects::FilterEffect::new()),
            "Eq5" => self.add_effect(effects::Eq5Effect::new()),
            "Delay" => self.add_effect(effects::DelayEffect::new()),
            "Reverb" => self.add_effect(effects::ReverbEffect::new()),
            "Chorus" => self.add_effect(effects::ChorusEffect::new()),
            "Compressor" => self.add_effect(effects::CompressorEffect::new_compressor()),
            "Gate" => self.add_effect(effects::GateEffect::new()),
            "Distortion" => self.add_effect(effects::DistortionEffect::new()),
            _ => {
                return Err(Error::ParameterError(format!(
                    "Unknown effect: {effect_name}"
                )))
            }
        }
    }

    // Add given effect instance to the synth mixer, returning effect ID and parameter metadata JSON
    pub fn add_effect<E: Effect>(&mut self, effect: E) -> Result<(EffectId, String), Error> {
        // Store parameter metadata
        let parameters = effect
            .parameters()
            .iter()
            .map(|p| p.dyn_clone())
            .collect::<Vec<_>>();
        let info_json = serde_json::to_string(&EffectInfo::from(&effect as &dyn Effect))
            .unwrap_or_else(|_| "{}".to_string());
        let effect_handle = self.player.add_effect(effect, self.synth_mixer.id())?;
        let effect_id = effect_handle.id();

        self.active_effects
            .insert(effect_id, (effect_handle, parameters));

        Ok((effect_id, info_json))
    }

    // Remove an effect
    pub fn remove_effect(&mut self, effect_id: EffectId) -> Result<(), Error> {
        if let Some((_, _)) = self.active_effects.remove(&effect_id) {
            self.player.remove_effect(effect_id)?;
            Ok(())
        } else {
            Err(Error::EffectNotFoundError(effect_id))
        }
    }

    // Convert an effect parameter's value to a string
    pub fn effect_parameter_value_to_string(
        &self,
        effect_id: EffectId,
        param_id: FourCC,
        normalized_value: f32,
    ) -> Result<String, Error> {
        // Get effect info
        let (_, parameters) = self
            .active_effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?;

        // Find the parameter
        let parameter =
            parameters
                .iter()
                .find(|p| p.id() == param_id)
                .ok_or(Error::ParameterError(format!(
                    "Parameter {:?} not found in effect {}",
                    param_id, effect_id
                )))?;

        // Convert to string using the parameter's method
        Ok(parameter.value_to_string(normalized_value, true))
    }

    // Convert an effect parameter's value from a string
    pub fn effect_parameter_string_to_value(
        &self,
        effect_id: EffectId,
        param_id: FourCC,
        string: String,
    ) -> Result<Option<f32>, Error> {
        // Get effect info
        let (_, parameters) = self
            .active_effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?;

        // Find the parameter
        let parameter =
            parameters
                .iter()
                .find(|p| p.id() == param_id)
                .ok_or(Error::ParameterError(format!(
                    "Parameter {:?} not found in effect {}",
                    param_id, effect_id
                )))?;

        // Convert string to f32 using the parameter's method
        Ok(parameter.string_to_value(string))
    }

    // Set an effect parameter value and update our tracking
    pub fn set_effect_parameter_value(
        &mut self,
        effect_id: EffectId,
        param_id: FourCC,
        normalized_value: f32,
    ) -> Result<(), Error> {
        let (effect_handle, _) = self
            .active_effects
            .get(&effect_id)
            .ok_or(Error::EffectNotFoundError(effect_id))?;

        effect_handle.set_parameter(
            (
                param_id,
                ParameterValueUpdate::Normalized(normalized_value.clamp(0.0, 1.0)),
            ),
            None,
        )
    }
}
