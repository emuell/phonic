//! An example showcasing the Sampler generator's granular synthesis and modulation.

use std::time::Duration;

use phonic::{
    generators::{
        GrainOverlapMode, GrainPlaybackDirection, GrainPlayheadMode, GrainWindowMode,
        GranularParameters, LfoWaveform, Sampler,
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

// Granular parameter consts (tweak as needed!)

/// Path to sample
const SAMPLE_PATH: &str = "assets/pad-ambient.wav"; // "assets/YuaiLoop.wav";

// AHDSR Envelope parameters
const ATTACK_MS: u64 = 100;
const HOLD_MS: u64 = 0;
const DECAY_MS: u64 = 0;
const SUSTAIN_LEVEL: f32 = 1.0;
const RELEASE_MS: u64 = 2000;

/// Grainular parameters
const GRAIN_OVERLAP_MODE: GrainOverlapMode = GrainOverlapMode::Cloud;
const GRAIN_WINDOW: GrainWindowMode = GrainWindowMode::Hann;
const GRAIN_SIZE: f32 = 80.0; // 1ms - 1000ms
const GRAIN_DENSITY: f32 = 40.0; // 1hz - 100hz
const GRAIN_VARIATION: f32 = 0.25; // 0.0 = no variation, 1.0 = full variation
const GRAIN_SPRAY: f32 = 0.2; // 0.0 = no randomness, 1.0 = full random
const GRAIN_PAN_SPREAD: f32 = 0.15; // 0.0 = no pan, 1.0 = full left/right
const GRAIN_PLAYBACK_DIR: GrainPlaybackDirection = GrainPlaybackDirection::Forward;
const GRAIN_PLAYHEAD_MODE: GrainPlayheadMode = GrainPlayheadMode::PlayThrough;
const GRAIN_MANUAL_POSITION: f32 = 0.5; // 0.0 = start, 0.5 = middle, 1.0 = end
const GRAIN_PLAYHEAD_SPEED: f32 = 2.0; // speed for PlayThrough mode (0.1 - 4.0)

// Modulation Parameters - LFO 1
const MOD_LFO1_RATE: f32 = 0.15; // Hz
const MOD_LFO1_WAVEFORM: LfoWaveform = LfoWaveform::SmoothRandom;
const MOD_LFO1_TO_GRAIN_POS: f32 = 0.4; // 40% modulation depth

// Modulation Parameters - LFO 2
const MOD_LFO2_RATE: f32 = 2.5; // Hz
const MOD_LFO2_WAVEFORM: LfoWaveform = LfoWaveform::Sine;
const MOD_LFO2_TO_GRAIN_SIZE: f32 = 0.3; // 30% modulation depth

// Modulation Parameters - Velocity
const MOD_VEL_TO_GRAIN_DENSITY: f32 = 0.5; // Louder notes = denser grains

// Modulation Parameters - Keytracking
const MOD_KEY_TO_GRAIN_SIZE: f32 = -0.2; // -20% per octave

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
    granular_parameters.overlap_mode = GRAIN_OVERLAP_MODE;
    granular_parameters.window = GRAIN_WINDOW;
    granular_parameters.size = GRAIN_SIZE;
    granular_parameters.density = GRAIN_DENSITY;
    granular_parameters.variation = GRAIN_VARIATION;
    granular_parameters.spray = GRAIN_SPRAY;
    granular_parameters.pan_spread = GRAIN_PAN_SPREAD;
    granular_parameters.playback_direction = GRAIN_PLAYBACK_DIR;
    granular_parameters.playhead_mode = GRAIN_PLAYHEAD_MODE;
    granular_parameters.manual_position = GRAIN_MANUAL_POSITION;
    granular_parameters.playhead_speed = GRAIN_PLAYHEAD_SPEED;

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

    println!("\nConfiguring modulation system...");

    // Configure LFO 1 source
    generator.set_parameter(Sampler::MOD_LFO1_RATE.value_update(MOD_LFO1_RATE), None)?;
    generator.set_parameter(
        Sampler::MOD_LFO1_WAVEFORM.value_update(MOD_LFO1_WAVEFORM),
        None,
    )?;
    // Route LFO 1 to Grain Position (unipolar application - adds to position)
    generator.set_modulation(
        Sampler::MOD_SOURCE_LFO1,
        Sampler::GRAIN_POSITION.id(),
        MOD_LFO1_TO_GRAIN_POS,
        false, // unipolar: LFO output used as-is
        None,
    )?;

    // Configure LFO 2 source
    generator.set_parameter(Sampler::MOD_LFO2_RATE.value_update(MOD_LFO2_RATE), None)?;
    generator.set_parameter(
        Sampler::MOD_LFO2_WAVEFORM.value_update(MOD_LFO2_WAVEFORM),
        None,
    )?;
    // Route LFO 2 to Grain Size (unipolar - only increases size)
    generator.set_modulation(
        Sampler::MOD_SOURCE_LFO2,
        Sampler::GRAIN_SIZE.id(),
        MOD_LFO2_TO_GRAIN_SIZE,
        false, // unipolar
        None,
    )?;

    // Route Velocity to Grain Density (unipolar - higher velocity = higher density)
    // Velocity and Keytracking sources are automatically available - no configuration needed
    generator.set_modulation(
        Sampler::MOD_SOURCE_VELOCITY,
        Sampler::GRAIN_DENSITY.id(),
        MOD_VEL_TO_GRAIN_DENSITY,
        false, // unipolar
        None,
    )?;

    // Route Keytracking to Grain Size (BIPOLAR - middle key neutral, upper/lower keys +/-)
    generator.set_modulation(
        Sampler::MOD_SOURCE_KEYTRACK,
        Sampler::GRAIN_SIZE.id(),
        MOD_KEY_TO_GRAIN_SIZE,
        true, // BIPOLAR: center key = no change, higher = +, lower = -
        None,
    )?;

    // Print DSP graph
    println!("\nPlayer Graph:\n{}", player);

    // Print parameters
    println!("\n=== Volume Envelope ===");
    println!("Attack: {ATTACK_MS}ms");
    println!("Hold: {HOLD_MS}ms");
    println!("Decay: {DECAY_MS}ms");
    println!("Release: {RELEASE_MS}ms");

    println!("\n=== Grain Parameters ===");
    println!("Overlap Mode: {GRAIN_OVERLAP_MODE}");
    println!("Window: {GRAIN_WINDOW}");
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
