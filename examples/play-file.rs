//! An example showcasing how to play audio files, both preloaded and streamed
//! and how to monitor file playback status.

use std::{sync::mpsc::sync_channel, time::Duration};

use phonic::{utils::speed_from_note, Error, FilePlaybackOptions, PlaybackStatusEvent};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert-allocs"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

// Common example code
#[path = "./common/arguments.rs"]
mod arguments;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Parse optional arguments
    let args = arguments::parse();

    // Create a player with the default output device and a channel to receive playback events.
    let (playback_status_sender, playback_status_receiver) = sync_channel(32);
    let mut player = arguments::new_player(&args, playback_status_sender)?;

    // Pause playback until we've added all sources.
    player.stop();

    // Create sound sources and memorize handles for control
    let playing_files = [
        player
            // files are by default not streamed but are preloaded and played buffered.
            .play_file(
                "assets/altijd synth bit.wav",
                FilePlaybackOptions::default(),
            )?,
        player
            // this file is going to be streamed on the fly with a lowered volume
            // and a fade out. also pan it a bit to the left and loop it 2 times.
            .play_file(
                "assets/YuaiLoop.wav",
                FilePlaybackOptions::default()
                    .streamed()
                    .volume_db(-2.0)
                    .panning(-0.3)
                    .speed(speed_from_note(58))
                    .repeat(2)
                    .fade_out(Duration::from_secs(3)),
            )?,
    ];

    // Stop (fade-out) the loop after 3 secs.
    let samples_per_second = player.output_sample_rate() as u64;
    let now = player.output_sample_frame_position();

    let loop_playback_handle = &playing_files[1];
    loop_playback_handle.stop(now + 3 * samples_per_second)?;

    // Handle events from the file sources.
    std::thread::spawn(move || {
        while let Ok(event) = playback_status_receiver.recv() {
            match event {
                PlaybackStatusEvent::Position {
                    id,
                    path,
                    context: _,
                    position,
                } => {
                    println!(
                        "Playback pos of file #{} '{}': {}",
                        id,
                        path,
                        position.as_secs_f32()
                    );
                }
                PlaybackStatusEvent::Stopped {
                    id,
                    path,
                    context: _,
                    exhausted,
                } => {
                    if exhausted {
                        println!("Playback of #{id} '{path}' finished playback");
                    } else {
                        println!("Playback of #{id} '{path}' was stopped");
                    }
                }
            }
        }
    });

    // Print DSP graph
    println!("\nPlayer Graph:\n{}", player);

    // Start playing.
    player.start();

    // Wait until all files sources finished playing...
    while playing_files.iter().any(|file| file.is_playing()) {
        std::thread::sleep(Duration::from_millis(100));
    }

    Ok(())
}
