//! An example to showcase and test concurrent mixer processing.
//!
//! Usage:
//!   cargo run --release --example play-multi-threaded --features fundsp -- [OPTIONS]
//!
//! Options:
//!   --threads <N>            Number of worker threads. Set to 0 or 1 to disable
//!                            concurrent processing. (default: CPU count)
//!   --submixers <N>          Number of submixers to create (default: 8)
//!   --sources-per-mixer <N>  Number of sources per submixer (default: 4)
//!   --effects-per-mixer <N>  Number of effects per submixer (default: 2)
//!   --duration <N>           Duration in seconds (default: 10)
//!   --seed <N>               Random seed for deterministic behavior
//!   -o, --output <PATH>      Write to WAV file instead of audio device
//!   -l, --log-level <LEVEL>  Set logging level (debug, info, warn, error)

use std::{ops::RangeInclusive, path::PathBuf, time::Duration};

use arg::{parse_args, Args};
use phonic::{
    effects::{CompressorEffect, GainEffect, ReverbEffect},
    four_cc::FourCC,
    generators::{FunDspGenerator, Sampler},
    outputs::WavOutput,
    parameters::FloatParameter,
    utils::{ahdsr::AhdsrParameters, fundsp::shared_ahdsr, pitch_from_note, speed_from_note},
    DefaultOutputDevice, Error, FilePlaybackOptions, GeneratorPlaybackOptions, Parameter, Player,
    PlayerConfig, SynthPlaybackOptions,
};
use rand::{rngs::StdRng, Rng, SeedableRng};

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

/// Arguments for the parallel mixer example.
#[derive(Args, Debug)]
struct ParallelArguments {
    #[arg(short = "o", long = "output")]
    /// Write audio output into the given wav file
    output_path: Option<PathBuf>,
    #[arg(short = "l", long = "log-level")]
    /// Set logging level
    log_level: Option<log::Level>,
    #[arg(long = "threads")]
    /// Number of worker threads. Set to 0 to disable. (default: CPU count)
    threads: Option<usize>,
    #[arg(long = "submixers")]
    /// Number of submixers to create (default: 8)
    submixers: Option<usize>,
    #[arg(long = "sources-per-mixer")]
    /// Number of sources per submixer (default: 4)
    sources_per_mixer: Option<usize>,
    #[arg(long = "effects-per-mixer")]
    /// Number of effects per submixer (default: 2)
    effects_per_mixer: Option<usize>,
    #[arg(long = "duration")]
    /// Duration in seconds (default: 10)
    duration: Option<u64>,
    #[arg(long = "seed")]
    /// Random seed for deterministic behavior
    seed: Option<usize>,
}

// -------------------------------------------------------------------------------------------------

// Get a random note from the c minor pentatonic scale in the given note range
fn random_note(valid_range: RangeInclusive<u8>, rng: &mut impl Rng) -> u8 {
    let scale_intervals = [0, 3, 5, 7, 10];

    let min_midi = *valid_range.start();
    let max_midi = *valid_range.end();

    // Find all valid C minor notes within the range
    let mut valid_notes = Vec::new();

    // MIDI note 0 is C-1, so we need to find octaves
    // Start from the lowest C at or below min_midi
    let lowest_c = (min_midi / 12) * 12; // Round down to nearest C

    // Generate all C minor notes in range
    let mut octave = lowest_c;
    while octave <= max_midi {
        for &interval in &scale_intervals {
            let note = octave + interval;
            if note >= min_midi && note <= max_midi {
                valid_notes.push(note);
            }
        }
        octave += 12; // Next octave
    }

    // Pick a random note from valid notes
    let random_index = rng.random_range(0..valid_notes.len());
    valid_notes[random_index]
}

// -------------------------------------------------------------------------------------------------

// Simple FunDSP generator parameters
const DETUNE: FloatParameter = FloatParameter::new(FourCC(*b"detn"), "Detune", 0.0..=1.0, 0.02);
const BRIGHTNESS: FloatParameter =
    FloatParameter::new(FourCC(*b"brit"), "Brightness", 0.0..=1.0, 0.5);

/// Voice factory for a simple polyphonic synth with FunDSP.
/// Creates a detuned oscillator voice with a simple filter.
fn simple_synth_voice_factory(
    gate: fundsp::prelude32::Shared,
    freq: fundsp::prelude32::Shared,
    volume: fundsp::prelude32::Shared,
    _panning: fundsp::prelude32::Shared,
    parameter: &mut dyn FnMut(FourCC) -> fundsp::prelude32::Shared,
) -> Box<dyn fundsp::prelude32::AudioUnit> {
    use fundsp::prelude32::*;

    let detune_amt = parameter(DETUNE.id());

    // Create envelope with shared parameters
    let env = shared_ahdsr(
        gate.clone(),
        shared(0.01),
        shared(0.0),
        shared(0.1),
        shared(0.7),
        shared(0.3),
    );

    // Two detuned oscillators
    let osc1 = var(&freq) >> saw();
    let osc2 = (var(&freq) * (1.0 + var(&detune_amt) * 0.02)) >> saw();
    let osc_mix = (osc1 + osc2) * 0.5;

    // Simple lowpass filter (static cutoff for simplicity)
    let filtered = osc_mix >> lowpole_hz(2000.0);

    // Apply envelope and volume
    let signal = filtered * env * var(&volume);

    // Center panning (static for simplicity) and convert to stereo
    Box::new(signal >> pan(0.0))
}

/// Get parameters for the simple synth generator
static SIMPLE_SYNTH_PARAMS: &[&dyn Parameter] = &[&DETUNE, &BRIGHTNESS];

// -------------------------------------------------------------------------------------------------

/// Create a simple sine wave with FunDSP
fn create_sine_synth(
    frequency: f32,
    amplitude: f32,
    duration_secs: f32,
) -> Box<dyn fundsp::prelude32::AudioUnit> {
    use fundsp::prelude32::*;

    let freq = shared(frequency);
    let amp = shared(amplitude);
    let fade_out_time = 0.5;

    let gate = envelope(move |t| {
        if t < duration_secs {
            1.0
        } else if t < duration_secs + fade_out_time {
            1.0 - (t - duration_secs) / fade_out_time
        } else {
            0.0
        }
    });

    // Return mono signal - will be automatically handled by phonic
    Box::new(gate * (var(&freq) >> sine()) * var(&amp))
}

/// Create a simple square wave with FunDSP
fn create_square_synth(
    frequency: f32,
    amplitude: f32,
    duration_secs: f32,
) -> Box<dyn fundsp::prelude32::AudioUnit> {
    use fundsp::prelude32::*;

    let freq = shared(frequency);
    let amp = shared(amplitude);
    let fade_out_time = 0.5;

    let gate = envelope(move |t| {
        if t < duration_secs {
            1.0
        } else if t < duration_secs + fade_out_time {
            1.0 - (t - duration_secs) / fade_out_time
        } else {
            0.0
        }
    });

    // Return mono signal
    Box::new(gate * (var(&freq) >> square()) * var(&amp))
}

/// Create pink noise with FunDSP
fn create_noise_synth(amplitude: f32, duration_secs: f32) -> Box<dyn fundsp::prelude32::AudioUnit> {
    use fundsp::prelude32::*;

    let amp = shared(amplitude);
    let fade_out_time = 0.5;

    let gate = envelope(move |t| {
        if t < duration_secs {
            1.0
        } else if t < duration_secs + fade_out_time {
            1.0 - (t - duration_secs) / fade_out_time
        } else {
            0.0
        }
    });

    // Return mono signal
    Box::new(gate * pink() * var(&amp))
}

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse arguments
    let args = parse_args::<ParallelArguments>();

    // Init logger
    arguments::create_logger(args.log_level);

    let submixer_count = args.submixers.unwrap_or(8);
    let sources_per_mixer = args.sources_per_mixer.unwrap_or(4);
    let effects_per_mixer = args.effects_per_mixer.unwrap_or(2);
    let duration_secs = args.duration.unwrap_or(10);

    // Initialize RNG
    let mut rng = if let Some(seed) = args.seed {
        StdRng::seed_from_u64(seed as u64)
    } else {
        StdRng::from_os_rng()
    };

    println!("=== Parallel Mixer Test ===");

    // Create player config with parallel processing settings
    let mut config = PlayerConfig::default().concurrent_processing(true);
    if let Some(thread_count) = args.threads {
        if thread_count > 1 {
            config = config.concurrent_worker_threads(thread_count);
        } else {
            config = config.concurrent_processing(false);
        }
    }

    // Create player with the configured settings
    let mut player: Player = if let Some(output_path) = &args.output_path {
        Player::new_with_config(WavOutput::open(output_path)?, None, config.clone())
    } else {
        Player::new_with_config(DefaultOutputDevice::open()?, None, config.clone())
    };

    // Stop player while we set up
    player.stop();

    // Lower master volume to avoid clipping
    player.set_output_volume(1.0 / submixer_count as f32 * 4.0);

    // Create submixers with sources and effects
    let mut submixers = Vec::new();

    for mixer_idx in 0..submixer_count {
        let mixer = player.add_mixer(None)?;
        submixers.push(mixer.id());

        // Add random sources to this submixer
        for source_idx in 0..sources_per_mixer {
            let frequency = pitch_from_note(random_note(12..=72, &mut rng)) as f32;
            let amplitude = 0.05; // Keep low to avoid clipping
            let duration = duration_secs as f32;
            match rng.random_range(0..7) {
                0 => {
                    // Sine wave synth
                    player.play_fundsp_synth(
                        &format!("sine_{}_{}", mixer_idx, source_idx),
                        create_sine_synth(frequency, amplitude, duration),
                        SynthPlaybackOptions::default().target_mixer(mixer.id()),
                    )?;
                }
                1 => {
                    // Square wave synth
                    player.play_fundsp_synth(
                        &format!("square_{}_{}", mixer_idx, source_idx),
                        create_square_synth(frequency, amplitude * 0.5, duration),
                        SynthPlaybackOptions::default().target_mixer(mixer.id()),
                    )?;
                }
                2 => {
                    // Noise synth
                    player.play_fundsp_synth(
                        &format!("noise_{}_{}", mixer_idx, source_idx),
                        create_noise_synth(amplitude * 0.3, duration),
                        SynthPlaybackOptions::default().target_mixer(mixer.id()),
                    )?;
                }
                3 => {
                    // File playback (preloaded)
                    let _handle = player.play_file(
                        "assets/altijd synth bit.wav",
                        FilePlaybackOptions::default()
                            .volume_db(-6.0)
                            .target_mixer(mixer.id())
                            .repeat(2),
                    )?;
                }
                4 => {
                    // File playback (streamed)
                    player.play_file(
                        "assets/YuaiLoop.wav",
                        FilePlaybackOptions::default()
                            .streamed()
                            .volume_db(-6.0)
                            .speed(speed_from_note(random_note(48..=72, &mut rng)))
                            .target_mixer(mixer.id())
                            .fade_out(Duration::from_secs(2)),
                    )?;
                }
                5 => {
                    // Sampler with AHDSR envelope
                    let sampler = Sampler::from_file(
                        "assets/bass.wav",
                        Some(AhdsrParameters::new(
                            Duration::from_millis(10),
                            Duration::ZERO,
                            Duration::ZERO,
                            1.0,
                            Duration::from_secs(1),
                        )?),
                        GeneratorPlaybackOptions::default()
                            .voices(4)
                            .target_mixer(mixer.id()),
                        player.output_channel_count(),
                        player.output_sample_rate(),
                    )?;
                    let generator = player.play_generator(sampler, None)?;

                    // Play a few random notes
                    let sample_rate = player.output_sample_rate() as u64;
                    let now = player.output_sample_frame_position();
                    for i in 0..4 {
                        let note = random_note(36..=60, &mut rng);
                        let time = now + (i * sample_rate);
                        generator.note_on(note, Some(0.5), None, time)?;
                    }
                }
                _ => {
                    // FunDSP generator (polyphonic synth)
                    let fundsp_gen = FunDspGenerator::with_parameters(
                        &format!("poly_synth_{}_{}", mixer_idx, source_idx),
                        SIMPLE_SYNTH_PARAMS,
                        None,
                        simple_synth_voice_factory,
                        GeneratorPlaybackOptions::default()
                            .voices(4)
                            .target_mixer(mixer.id()),
                        player.output_sample_rate(),
                    )?;

                    let generator = player.play_generator(fundsp_gen, None)?;

                    // Play a few random notes with different timings
                    let sample_rate = player.output_sample_rate() as u64;
                    let now = player.output_sample_frame_position();
                    for i in 0..3 {
                        let note = random_note(48..=72, &mut rng);
                        let time = now + (i * sample_rate / 2); // Half second apart
                        generator.note_on(note, Some(0.6), None, time)?;
                    }
                }
            }
        }

        // Add random effects to this submixer
        for _effect_idx in 0..effects_per_mixer {
            match rng.random_range(0..3) {
                0 => {
                    let gain_db = rng.random_range(-6.0..0.0);
                    player.add_effect(GainEffect::with_gain_db(gain_db), mixer.id())?;
                }
                1 => {
                    let wet = rng.random_range(0.2..0.5);
                    let decay = rng.random_range(0.3..0.7);
                    player.add_effect(ReverbEffect::with_parameters(wet, decay), mixer.id())?;
                }
                _ => {
                    player.add_effect(CompressorEffect::new_limiter(), mixer.id())?;
                }
            }
        }
    }

    println!();
    println!("Player Graph:");
    println!("{}", player);
    println!();

    println!("Submixers: {submixer_count}");
    println!("Sources per mixer: {sources_per_mixer}");
    println!("Effects per mixer: {effects_per_mixer}");
    println!("Concurrent processing: {}", config.concurrent_processing);
    if config.concurrent_processing {
        let effective_thread_count = config.effective_concurrent_worker_threads();
        println!("Worker threads: {effective_thread_count}");
    }
    println!();

    // Start playback
    println!("Starting playback...");
    println!();
    player.start();

    // Monitor playback and print stats
    let start_time = std::time::Instant::now();
    let mut last_stats_time = start_time;

    while start_time.elapsed().as_secs() < duration_secs && player.is_running() {
        std::thread::sleep(Duration::from_millis(500));

        // Print stats every 2 seconds
        if last_stats_time.elapsed().as_secs() >= 2 {
            let elapsed = start_time.elapsed().as_secs();
            let cpu_load = player.cpu_load();

            println!(
                "[{:2}s] CPU: {:.1}% (peak: {:.1}%)",
                elapsed,
                cpu_load.average * 100.0,
                cpu_load.peak * 100.0
            );

            last_stats_time = std::time::Instant::now();
        }
    }

    player.stop_all_sources()?;
    std::thread::sleep(Duration::from_secs(1));

    println!();
    println!("Playback finished.");
    Ok(())
}
