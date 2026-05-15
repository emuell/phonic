//! An example showcasing how to play a musical sequence using the Pattern and Metronome
//! sequencers with preloaded audio samples and glide playback parameters.

use std::time::Duration;

use phonic::{
    generators::{Metronome, Pattern, PatternEvent, Sampler, Sequencer, SequencerTransport},
    utils::ahdsr::AhdsrParameters,
    Error, GeneratorPlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
mod common;
use common::arguments;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, None)?;

    // Stop playback until we've scheduled all notes
    player.stop();

    // Create metronome sampler
    let mut metronome = player.play_generator(
        Sampler::from_file(
            "assets/cowbell.wav",
            GeneratorPlaybackOptions::default().voices(2),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None,
    )?;

    // Create bass sampler
    let mut bass = player.play_generator(
        Sampler::from_file(
            "assets/bass.wav",
            GeneratorPlaybackOptions::default(),
            player.output_channel_count(),
            player.output_sample_rate(),
        )?
        .with_ahdsr(AhdsrParameters::new(
            Duration::from_millis(10),
            Duration::ZERO,
            Duration::ZERO,
            1.0,
            Duration::from_secs_f32(2.0),
        )?)?,
        None,
    )?;

    // Sequencer timing
    const BARS_TO_PLAY: usize = 4;
    let samples_per_sec = player.output_sample_rate() as u64;
    let output_start_time = player.output_sample_frame_position() + samples_per_sec;

    let transport = SequencerTransport::new(player.output_sample_rate(), 120.0, 4);
    
    // Schedule metronome beats
    let mut metro_seq = Metronome::new(BARS_TO_PLAY, output_start_time, transport);
    metro_seq.run_until(u64::MAX, &mut metronome);
    metronome.stop(
        output_start_time
            + BEATS_PER_BAR as u64 * BARS_TO_PLAY as u64 * transport.samples_per_beat(),
    )?;

    // Schedule bass line with glides using the Pattern sequencer
    let bass_notes = vec![
        PatternEvent::note_on(48 + 12, 3.0).panning(0.0),
        PatternEvent::note_on(36 + 12, 0.5)
            .glide(999.0)
            .panning(0.0),
        PatternEvent::note_on(48 + 12, 0.5)
            .glide(999.0)
            .panning(0.0),
        PatternEvent::note_on(44 + 12, 1.0)
            .glide(60.0)
            .volume(0.75)
            .panning(0.8),
        PatternEvent::note_on(46 + 12, 1.0)
            .glide(60.0)
            .volume(0.5)
            .panning(-0.8),
        PatternEvent::note_on(53 + 12, 2.0).glide(12.0).panning(0.0),
        PatternEvent::note_on(44 + 12, 4.0).glide(60.0).volume(1.0),
    ];

    let mut bass_seq = Pattern::new(bass_notes, output_start_time, 1, transport);
    bass_seq.run_until(u64::MAX, &mut bass);
    bass.stop((BEATS_PER_BAR * BARS_TO_PLAY + 1) as u64 * transport.samples_per_beat())?;

    // Print player graph
    println!("\nPlayer Graph:\n{}", player);

    // Start playback
    player.start();

    // Wait for playback to finish
    while player.is_running() && (bass.is_playing() || metronome.is_playing()) {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
