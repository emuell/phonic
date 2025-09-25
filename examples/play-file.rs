//! An example showcasing how to play audio files, both preloaded and streamed
//! and how to monitor file playback status.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use phonic::{utils::speed_from_note, Error, FilePlaybackOptions, PlaybackStatusEvent};

#[cfg(feature = "time-stretching")]
use phonic::TimeStretchMode;

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
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

    // Create a channel for playback status events.
    // NB: prefer using a bounded channel here to avoid memory allocations in the audio thread.
    let (status_sender, status_receiver) = crossbeam_channel::bounded(32);

    // Create a player instance with the output device as configured via program arguments
    let mut player = arguments::new_player(&args, status_sender)?;

    // pause playback until we've added all sources
    player.stop();

    // create sound sources and memorize file ids for the playback status
    let mut playing_file_ids = vec![
        // files are by default not streamed but are preloaded and played buffered.
        player.play_file(
            "assets/altijd synth bit.wav",
            FilePlaybackOptions::default(),
        )?,
        // this file is going to be streamed on the fly, looped and played back
        // with a lowered volume, custom panning and a fade out.
        // when `time-stretching` feature is enabled this also stretches the playback
        // speed, else it's repeating the sample loop twice.
        #[cfg(feature = "time-stretching")]
        player.play_file(
            "assets/YuaiLoop.wav",
            FilePlaybackOptions::default()
                .streamed()
                .volume_db(-2.0)
                .panning(-0.3)
                .speed(speed_from_note(58))
                .stretch(0.5)
                .stretch_mode(TimeStretchMode::SignalSmithDefault)
                .fade_out(Duration::from_secs(3)),
        )?,
        #[cfg(not(feature = "time-stretching"))]
        player.play_file(
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

    let mut loop_playback_id = Some(playing_file_ids[1]);
    let is_running = Arc::new(AtomicBool::new(true));

    // handle events from the file sources
    let event_thread = std::thread::spawn({
        let is_running = is_running.clone();
        move || {
            while let Ok(event) = status_receiver.recv() {
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
                        playing_file_ids.retain(|v| *v != id);
                        if playing_file_ids.is_empty() {
                            // stop thread when all files finished
                            is_running.store(false, Ordering::Relaxed);
                            break;
                        }
                    }
                }
            }
        }
    });

    // start playing
    player.start();

    // stop (fade-out) the loop after 8 secs
    let samples_per_sec = player.output_sample_rate() as u64;
    let start_time = player.output_sample_frame_position();
    while is_running.load(Ordering::Relaxed) {
        let current_time = player.output_sample_frame_position();
        let elapsed = (current_time - start_time) / samples_per_sec;
        if loop_playback_id.is_some() && elapsed > 8 {
            player.stop_source(loop_playback_id.unwrap(), None)?;
            loop_playback_id = None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // wait until playback finished
    event_thread.join().map_err(|_| Error::SendError)?;

    Ok(())
}
