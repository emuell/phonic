//! An example showcasing how to play audio files, both preloaded and streamed
//! and how to monitor file playback status.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use phonic::{
    utils::speed_from_note, DefaultOutputDevice, Error, FilePlaybackOptions, OutputDevice,
    PlaybackStatusEvent, Player,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Open default device
    let audio_output = DefaultOutputDevice::open()?;

    // create channel for playback status events
    // Prefer using a bounded channel here to avoid memory allocations in the audio thread.
    let (status_sender, status_receiver) = crossbeam_channel::bounded(32);
    let mut player = Player::new(audio_output.sink(), Some(status_sender));

    // pause playing until we've added all sources
    player.stop();

    // create sound sources and memorize file ids for the playback status
    let mut playing_file_ids = vec![
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

    let mut loop_playback_id = Some(playing_file_ids[1]);
    let is_running = Arc::new(AtomicBool::new(true));

    // start playing
    player.start();

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

    // stop (fade-out) the loop after 3 secs
    let play_time = Instant::now();
    while is_running.load(Ordering::Relaxed) {
        if loop_playback_id.is_some() && play_time.elapsed() > Duration::from_secs(3) {
            player.stop_source(loop_playback_id.unwrap(), None)?;
            loop_playback_id = None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }

    // wait until playback thread finished
    event_thread.join().unwrap();

    Ok(())
}
