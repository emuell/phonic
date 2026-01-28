//! An example showcasing granular synthesis with modulation using the Sampler generator.
//!
//! Demonstrates how to use the granular playback mode of the Sampler with the new
//! modulation system to create interesting textures and effects from audio samples through
//! grain manipulation with LFO, envelope, velocity, and keytracking modulation sources.
//! It plays a C major chord with per-voice modulation.

use std::time::Duration;

use phonic::{
    generators::{
        GrainPlaybackDirection, GrainPlayheadMode, GrainWindowMode, GranularParameters,
        LfoWaveform, Sampler,
    },
    utils::ahdsr::AhdsrParameters,
    Error, GeneratorPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Granular synthesis parameter consts (tweak as needed!)

/// Path to sample
const SAMPLE_PATH: &str = "assets/pad-ambient.wav"; // "assets/YuaiLoop.wav";

/// Grain window type
const GRAIN_WINDOW: GrainWindowMode = GrainWindowMode::Hann;
/// Grain size in seconds
const GRAIN_SIZE: f32 = 80.0;
/// Grain density in Hz - controls how many grains spawn per second
const GRAIN_DENSITY: f32 = 40.0;
/// Grain variation (0.0 = no variation, 1.0 = full variation of size and volume)
const GRAIN_VARIATION: f32 = 0.25;
/// Grain spray randomness (0.0 = no randomness, 1.0 = full random)
const GRAIN_SPRAY: f32 = 0.2;
/// Pan spread per grain (0.0 = no pan, 1.0 = full left/right)
const GRAIN_PAN_SPREAD: f32 = 0.15;
/// Grain playback direction
const GRAIN_PLAYBACK_DIR: GrainPlaybackDirection = GrainPlaybackDirection::Forward;
/// Playhead mode (manual or playthrough)
const GRAIN_PLAYHEAD_MODE: GrainPlayheadMode = GrainPlayheadMode::PlayThrough;
/// Manual position in file when playhead mode is Manual (0.0 = start, 0.5 = middle, 1.0 = end)
const GRAIN_MANUAL_POSITION: f32 = 0.5;
/// Playback speed for PlayThrough mode (0.1 - 4.0)
/// Controls how fast grains move through the file in PlayThrough mode
const GRAIN_PLAYHEAD_SPEED: f32 = 2.0;

// Modulation Parameters - LFO 1 (Slow, smooth position modulation)
const MOD_LFO1_RATE: f32 = 0.15; // Hz
const MOD_LFO1_WAVEFORM: LfoWaveform = LfoWaveform::SmoothRandom;
const MOD_LFO1_TO_GRAIN_POS: f32 = 0.4; // 40% modulation depth

// Modulation Parameters - LFO 2 (Faster size modulation)
const MOD_LFO2_RATE: f32 = 2.5; // Hz
const MOD_LFO2_WAVEFORM: LfoWaveform = LfoWaveform::Sine;
const MOD_LFO2_TO_GRAIN_SIZE: f32 = 0.3; // 30% modulation depth

// Modulation Parameters - Velocity (affects grain density)
const MOD_VEL_TO_GRAIN_DENSITY: f32 = 0.5; // Louder notes = denser grains

// Modulation Parameters - Keytracking (higher notes = smaller grains)
const MOD_KEY_TO_GRAIN_SIZE: f32 = -0.2; // -20% per octave

// AHDSR Envelope parameters for voice playback (amplitude)
const ATTACK_MS: u64 = 100;
const HOLD_MS: u64 = 0;
const DECAY_MS: u64 = 0;
const SUSTAIN_LEVEL: f32 = 1.0;
const RELEASE_MS: u64 = 2000;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player with the default output device
    let mut player = arguments::new_player(&args, None)?;

    // Pause playback until we've added all sources.
    player.stop();

    let sample_rate = player.output_sample_rate();
    let channel_count = player.output_channel_count();

    // Create AHDSR envelope parameters for granular texture
    let ahdsr_params = AhdsrParameters::new(
        Duration::from_millis(ATTACK_MS),
        Duration::from_millis(HOLD_MS),
        Duration::from_millis(DECAY_MS),
        SUSTAIN_LEVEL,
        Duration::from_millis(RELEASE_MS),
    )?;

    // Create granular parameters
    let mut granular_parameters = GranularParameters::new();
    granular_parameters.grain_window = GRAIN_WINDOW;
    granular_parameters.grain_size = GRAIN_SIZE;
    granular_parameters.grain_density = GRAIN_DENSITY;
    granular_parameters.grain_variation = GRAIN_VARIATION;
    granular_parameters.grain_spray = GRAIN_SPRAY;
    granular_parameters.grain_pan_spread = GRAIN_PAN_SPREAD;
    granular_parameters.playback_direction = GRAIN_PLAYBACK_DIR;
    granular_parameters.playhead_mode = GRAIN_PLAYHEAD_MODE;
    granular_parameters.manual_position = GRAIN_MANUAL_POSITION;
    granular_parameters.playhead_speed = GRAIN_PLAYHEAD_SPEED;
    // Note: Old built-in LFO parameters removed - now using modulation system instead!

    // Create sampler with granular playback
    let sampler = Sampler::from_file(
        SAMPLE_PATH,
        None,
        GeneratorPlaybackOptions::default().voices(8),
        channel_count,
        sample_rate,
    )?
    .with_ahdsr(ahdsr_params)?
    .with_granular_playback(granular_parameters)?;

    let generator = player.play_generator(sampler, None)?;

    // Configure modulation parameters
    println!("\nConfiguring modulation system...");

    // LFO 1: Smooth position modulation
    generator.set_parameter(Sampler::MOD_LFO1_RATE.value_update(MOD_LFO1_RATE), None)?;
    generator.set_parameter(
        Sampler::MOD_LFO1_WAVEFORM.value_update(MOD_LFO1_WAVEFORM),
        None,
    )?;
    generator.set_parameter(
        Sampler::MOD_LFO1_TO_GRAIN_POS.value_update(MOD_LFO1_TO_GRAIN_POS),
        None,
    )?;

    // LFO 2: Fast size modulation
    generator.set_parameter(Sampler::MOD_LFO2_RATE.value_update(MOD_LFO2_RATE), None)?;
    generator.set_parameter(
        Sampler::MOD_LFO2_WAVEFORM.value_update(MOD_LFO2_WAVEFORM),
        None,
    )?;
    generator.set_parameter(
        Sampler::MOD_LFO2_TO_GRAIN_SIZE.value_update(MOD_LFO2_TO_GRAIN_SIZE),
        None,
    )?;

    // Velocity modulation: affects grain density
    generator.set_parameter(
        Sampler::MOD_VEL_TO_GRAIN_DENSITY.value_update(MOD_VEL_TO_GRAIN_DENSITY),
        None,
    )?;

    // Keytracking: higher notes have smaller grains
    generator.set_parameter(
        Sampler::MOD_KEY_TO_GRAIN_SIZE.value_update(MOD_KEY_TO_GRAIN_SIZE),
        None,
    )?;

    // Print DSP graph
    println!("\nPlayer Graph:\n{}", player);

    // Print grain parameters
    println!("\n=== Grain Parameters ===");
    println!("Grain Size: {GRAIN_SIZE:.1}ms");
    println!("Density: {GRAIN_DENSITY:.1} Hz");
    println!("Variation: {GRAIN_VARIATION:.2} (randomness)");
    println!("Spray: {GRAIN_SPRAY:.2} (randomness)");
    println!("Pan Spread: {GRAIN_PAN_SPREAD:.2} (stereo width)");
    println!("Playback Direction: {GRAIN_PLAYBACK_DIR}");
    println!("Playhead Mode: {GRAIN_PLAYHEAD_MODE}");
    if GRAIN_PLAYHEAD_MODE == GrainPlayheadMode::Manual {
        println!("Manual Position: {GRAIN_MANUAL_POSITION:.2}");
    } else {
        println!("Playhead Speed: {GRAIN_PLAYHEAD_SPEED:.2}x");
    }

    println!("\n=== Modulation System ===");
    println!("LFO 1: {MOD_LFO1_WAVEFORM} @ {MOD_LFO1_RATE:.2} Hz");
    println!("  → Grain Position: {:.0}%", MOD_LFO1_TO_GRAIN_POS * 100.0);
    println!("LFO 2: {MOD_LFO2_WAVEFORM} @ {MOD_LFO2_RATE:.2} Hz");
    println!("  → Grain Size: {:.0}%", MOD_LFO2_TO_GRAIN_SIZE * 100.0);
    println!("Velocity:");
    println!(
        "  → Grain Density: {:.0}%",
        MOD_VEL_TO_GRAIN_DENSITY * 100.0
    );
    println!("Keytracking:");
    println!("  → Grain Size: {:.0}%", MOD_KEY_TO_GRAIN_SIZE * 100.0);

    println!("\n=== Volume Envelope ===");
    println!("Attack: {ATTACK_MS}ms");
    println!("Hold: {HOLD_MS}ms");
    println!("Decay: {DECAY_MS}ms");
    println!("Release: {RELEASE_MS}ms");
    println!();

    // Start playing.
    player.start();

    // Play notes with 0.5 second spacing to hear different grain patterns
    // Use different velocities to demonstrate velocity modulation
    let now = player.output_sample_frame_position();
    let notes = [
        (60, 0.4), // C4 - soft (lower density due to velocity mod)
        (64, 0.7), // E4 - medium (medium density)
        (67, 1.0), // G4 - loud (higher density due to velocity mod)
    ];
    for (i, (note, velocity)) in notes.iter().enumerate() {
        let time = now + (i as u64 * sample_rate as u64 / 2); // 500ms apart
        generator.note_on(*note, Some(*velocity), None, time)?;
        println!(
            "Triggered note {} (vel: {:.1}) at +{:.1}s",
            note,
            velocity,
            i as f32 * 0.5
        );
    }

    // Stop all notes
    let stop_time = 12.0;
    generator.stop(now + (stop_time * sample_rate as f64) as u64)?;
    println!("Stop all notes at +{:.1}s", stop_time);

    // Keep playing until all notes finish
    while generator.is_playing() {
        std::thread::sleep(Duration::from_millis(100));
    }
    println!("Playback finished");

    Ok(())
}
