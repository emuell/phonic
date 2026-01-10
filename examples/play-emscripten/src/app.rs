use std::{cell::RefCell, collections::HashMap};

use emscripten_rs_sys::emscripten_request_animation_frame_loop;
use four_cc::FourCC;
use serde::Serialize;

use phonic::{
    effects,
    generators::FunDspGenerator,
    sources::PreloadedFileSource,
    utils::{db_to_linear, speed_from_note},
    DefaultOutputDevice, Effect, EffectHandle, EffectId, Error, FilePlaybackOptions,
    GeneratorPlaybackHandle, GeneratorPlaybackOptions, MixerHandle, NotePlaybackId, Parameter,
    ParameterType, ParameterValueUpdate, Player,
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

// -------------------------------------------------------------------------------------------------

// FunDSP synth impls

#[path = "../../common/synths/dx7.rs"]
mod dx7;
#[path = "../../common/synths/fm3.rs"]
mod fm3;
#[path = "../../common/synths/organ.rs"]
mod organ;
#[path = "../../common/synths/sub3.rs"]
mod sub3;

#[derive(Clone, Copy, PartialEq)]
enum SynthType {
    Sub3,
    Fm3,
    Organ,
    Dx7,
}

impl SynthType {
    fn info(&self) -> SynthInfo {
        match self {
            SynthType::Sub3 => SynthInfo::new("sub3", sub3::parameters()),
            SynthType::Fm3 => SynthInfo::new("fm3", fm3::parameters()),
            SynthType::Dx7 => SynthInfo::new("dx7", dx7::parameters()),
            SynthType::Organ => SynthInfo::new("organ", organ::parameters()),
        }
    }

    fn create_generator(
        &self,
        sample_rate: u32,
        voice_count: usize,
    ) -> Result<FunDspGenerator, Error> {
        let volume_db = -3.0;

        match self {
            Self::Sub3 => FunDspGenerator::with_parameters(
                "sub3_synth",
                sub3::parameters(),
                None,
                sub3::voice_factory,
                GeneratorPlaybackOptions::default()
                    .voices(voice_count)
                    .volume_db(volume_db),
                sample_rate,
            ),
            Self::Fm3 => FunDspGenerator::with_parameters(
                "fm3_synth",
                fm3::parameters(),
                None,
                fm3::voice_factory,
                GeneratorPlaybackOptions::default()
                    .voices(voice_count)
                    .volume_db(volume_db),
                sample_rate,
            ),
            Self::Organ => FunDspGenerator::with_parameters(
                "organ_synth",
                organ::parameters(),
                None,
                organ::voice_factory,
                GeneratorPlaybackOptions::default()
                    .voices(voice_count)
                    .volume_db(volume_db),
                sample_rate,
            ),
            Self::Dx7 => FunDspGenerator::with_parameters(
                "dx7_synth",
                dx7::parameters(),
                None,
                dx7::voice_factory,
                GeneratorPlaybackOptions::default()
                    .voices(voice_count)
                    .volume_db(volume_db),
                sample_rate,
            ),
        }
    }

    fn parameters(&self) -> &[&dyn Parameter] {
        match self {
            SynthType::Sub3 => sub3::parameters(),
            SynthType::Fm3 => fm3::parameters(),
            SynthType::Dx7 => dx7::parameters(),
            SynthType::Organ => organ::parameters(),
        }
    }

    fn randomize(&self) -> Vec<(FourCC, f32)> {
        match self {
            Self::Sub3 => sub3::randomize(),
            Self::Fm3 => fm3::randomize(),
            Self::Organ => organ::randomize(),
            Self::Dx7 => dx7::randomize(),
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
    playback_beat_counter: u32,
    playback_start_time: u64,
    metronome_enabled: bool,
    voice_count: usize,
    active_synth: SynthType,
    synth_handle: GeneratorPlaybackHandle,
    playing_notes: HashMap<u8, NotePlaybackId>,
    samples: Vec<PreloadedFileSource>,
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
        let sample_rate = player.output_sample_rate();

        // lower master volume a bit
        player.set_output_volume(db_to_linear(-3.0));

        // start playback in a second from now
        let playback_start_time =
            player.output_sample_frame_position() + player.output_sample_rate() as u64;
        let playback_beat_counter = 0;
        let metronome_enabled = true;

        println!("Creating synths...");
        // create a new mixer for the synth
        let synth_mixer = player.add_mixer(None)?;
        // create and add the initial synth (Sub3)
        let voice_count = 8;
        let active_synth = SynthType::Sub3;
        let synth_handle = player.add_generator(
            active_synth.create_generator(sample_rate, voice_count)?,
            synth_mixer.id(),
        )?;
        let active_effects = HashMap::new();

        println!("Preloading sample files...");
        let mut samples = Vec::new();
        for sample in ["./assets/cowbell.wav", "./assets/bass.wav"] {
            match PreloadedFileSource::from_file(
                sample,
                FilePlaybackOptions::default(),
                sample_rate,
            ) {
                Ok(sample) => samples.push(sample),
                Err(err) => return Err(err),
            }
        }

        let playing_notes = HashMap::new();

        println!("Start running...");
        unsafe {
            emscripten_request_animation_frame_loop(Some(Self::run_frame), std::ptr::null_mut())
        };

        Ok(Self {
            player,
            playback_start_time,
            playback_beat_counter,
            metronome_enabled,
            voice_count,
            synth_mixer,
            active_synth,
            synth_handle,
            playing_notes,
            samples,
            active_effects,
        })
    }

    // Player's current average CPU load
    pub fn cpu_load(&self) -> f32 {
        self.player.cpu_load().average
    }

    // Set the active synth type
    pub fn set_active_synth(&mut self, synth_type: i32) -> Result<(), Error> {
        let new_synth_type = match synth_type {
            0 => SynthType::Sub3,
            1 => SynthType::Fm3,
            2 => SynthType::Organ,
            3 | _ => SynthType::Dx7,
        };
        if new_synth_type != self.active_synth {
            self.active_synth = new_synth_type;
            // Stop all playing notes
            self.playing_notes.clear();
            // Remove the old synth
            self.player.remove_generator(self.synth_handle.id())?;
            // Create and add the new synth
            let sample_rate = self.player.output_sample_rate();
            self.synth_handle = self.player.add_generator(
                new_synth_type.create_generator(sample_rate, self.voice_count)?,
                self.synth_mixer.id(),
            )?;
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
            let sample_rate = self.player.output_sample_rate();
            self.synth_handle = self.player.add_generator(
                self.active_synth
                    .create_generator(sample_rate, voice_count)?,
                self.synth_mixer.id(),
            )?;
        }
        Ok(())
    }

    // Set metronome enabled state
    pub fn set_metronome_enabled(&mut self, enabled: bool) {
        self.metronome_enabled = enabled;
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
        let parameter = self
            .active_synth
            .parameters()
            .iter()
            .find(|p| p.id() == param_id)
            .ok_or(Error::ParameterError(format!(
                "Parameter {param_id} not found in active synth",
            )))?;

        Ok(parameter.value_to_string(normalized_value, true))
    }

    // Convert a synth parameter's string to a value
    pub fn synth_parameter_string_to_value(
        &self,
        param_id: FourCC,
        string: String,
    ) -> Result<Option<f32>, Error> {
        let parameter = self
            .active_synth
            .parameters()
            .iter()
            .find(|p| p.id() == param_id)
            .ok_or(Error::ParameterError(format!(
                "Parameter {param_id} not found in active synth",
            )))?;

        Ok(parameter.string_to_value(string))
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
    pub fn synth_note_on(&mut self, note: u8) {
        // If a note is already playing for this key, stop it first.
        if let Some(note_id) = self.playing_notes.remove(&note) {
            let _ = self.synth_handle.note_off(note_id, None);
        }

        // Trigger the new note with velocity 0.3 and default panning
        if let Ok(note_id) = self.synth_handle.note_on(note, Some(0.3), None, None) {
            self.playing_notes.insert(note, note_id);
        }
    }

    // Trigger a synth note off.
    pub fn synth_note_off(&mut self, note: u8) {
        if let Some(note_id) = self.playing_notes.remove(&note) {
            let _ = self.synth_handle.note_off(note_id, None);
        }
    }

    // Randomize synth parameters
    pub fn randomize_synth(&self) -> Vec<ParamUpdate> {
        let mut result = Vec::new();
        for (id, value) in self.active_synth.randomize() {
            use ParameterValueUpdate::Normalized;
            if self
                .synth_handle
                .set_parameter((id, Normalized(value)), None)
                .is_ok()
            {
                result.push(ParamUpdate {
                    id: id.into(),
                    value,
                });
            }
        }
        result
    }

    // Get list of available effects
    pub fn get_available_effects() -> Vec<String> {
        vec![
            "Gain".to_string(),
            "DcFilter".to_string(),
            "Filter".to_string(),
            "Eq5".to_string(),
            "Reverb".to_string(),
            "Chorus".to_string(),
            "Compressor".to_string(),
            "Distortion".to_string(),
        ]
    }

    // Add an effect by name to the synth mixer, returning effect ID and parameter metadata JSON
    pub fn add_effect_with_name(&mut self, effect_name: &str) -> Result<(EffectId, String), Error> {
        match effect_name {
            "Gain" => self.add_effect(effects::GainEffect::new()),
            "DcFilter" => self.add_effect(effects::DcFilterEffect::new()),
            "Filter" => self.add_effect(effects::FilterEffect::new()),
            "Eq5" => self.add_effect(effects::Eq5Effect::new()),
            "Reverb" => self.add_effect(effects::ReverbEffect::new()),
            "Chorus" => self.add_effect(effects::ChorusEffect::new()),
            "Compressor" => self.add_effect(effects::CompressorEffect::new_compressor()),
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

    // Animation frame callback which drives the player
    extern "C" fn run_frame(_time: f64, _user_data: *mut std::ffi::c_void) -> bool {
        APP.with_borrow_mut(|app| {
            // is a player running?
            if let Some(app) = app {
                app.run();
                true // continue running
            } else {
                false // stop running
            }
        })
    }

    // Schedule samples for playback
    fn run(&mut self) {
        // time consts
        const BEATS_PER_MIN: f64 = 120.0;
        const BEATS_PER_BAR: u32 = 4;

        // calculate metronome speed and signature
        let sample_rate = self.player.output_sample_rate();
        let samples_per_sec = self.player.output_sample_rate();
        let samples_per_beat = samples_per_sec as f64 * 60.0 / BEATS_PER_MIN;

        // schedule playback 0.5 seconds ahead of the players current time
        let preroll_time = (samples_per_sec as u64) / 2;
        let output_sample_time = self.player.output_sample_frame_position();

        // Calculate when the currently tracked beat is supposed to happen
        let mut next_beats_sample_time = (self.playback_start_time as f64
            + self.playback_beat_counter as f64 * samples_per_beat)
            as u64;

        // If we are lagging behind (the beat time is in the past): skip beats
        if next_beats_sample_time < output_sample_time {
            let elapsed = output_sample_time.saturating_sub(self.playback_start_time);
            // Calculate the beat index that corresponds to the next beat in the future
            let next_beat_index = (elapsed as f64 / samples_per_beat).ceil() as u32;
            if next_beat_index > self.playback_beat_counter {
                // println!("Skipping beats: {} -> {}", self.playback_beat_counter, next_beat_index);
                self.playback_beat_counter = next_beat_index;
                // Recalculate time for the new counter
                next_beats_sample_time = (self.playback_start_time as f64
                    + self.playback_beat_counter as f64 * samples_per_beat)
                    as u64;
            }
        }

        // schedule next sample when it's due within the preroll time, else do nothing
        if next_beats_sample_time < output_sample_time + preroll_time {
            // println!("Scheduling metronome sample to: {next_beats_sample_time}");

            if self.metronome_enabled {
                // play an octave higher every new bar start
                let sample_speed = speed_from_note(
                    if self.playback_beat_counter.is_multiple_of(BEATS_PER_BAR) {
                        72
                    } else {
                        60
                    },
                );
                // select a new sample every 2 bars
                let sample_index = (self.playback_beat_counter / (2 * BEATS_PER_BAR)) as usize
                    % self.samples.len();
                // clone the preloaded sample
                let sample = self.samples[sample_index]
                    .clone(
                        FilePlaybackOptions::default().speed(sample_speed),
                        sample_rate,
                    )
                    .unwrap();

                // play it at the new beat's time
                if let Ok(playback_handle) = self
                    .player
                    .play_file_source(sample, Some(next_beats_sample_time))
                {
                    // and stop it again (fade out) before the next beat starts
                    let _ = playback_handle.stop(next_beats_sample_time + samples_per_beat as u64);
                }
            }

            // advance beat counter
            self.playback_beat_counter += 1;
        }
    }
}
