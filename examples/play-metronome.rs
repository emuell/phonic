use std::time::Duration;

use afplay::{
    source::file::preloaded::PreloadedFileSource, utils::speed_from_note, AudioFilePlayer,
    AudioOutput, DefaultAudioOutput, Error, FilePlaybackOptions,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // create a player
    let mut player = AudioFilePlayer::new(DefaultAudioOutput::open()?.sink(), None);
    let sample_rate = player.output_sample_rate();

    // preload our metronome and bass sample
    let metronome_sample = PreloadedFileSource::new(
        "assets/cowbell.wav",
        None,
        FilePlaybackOptions::default(),
        sample_rate,
    )?;
    let bass_sample = PreloadedFileSource::new(
        "assets/bass.wav",
        None,
        FilePlaybackOptions::default(),
        sample_rate,
    )?;

    // define our metronome speed and signature
    let beats_per_min = 120.0;
    let beats_per_bar = 4;
    let samples_per_sec = player.output_sample_rate();
    let samples_per_beat = samples_per_sec as f64 * 60.0 / beats_per_min;
    let samples_to_seconds = |samples: u64| -> f64 { samples as f64 / samples_per_sec as f64 };

    // play 8 bars in this example, starting at the player's current playback pos
    const BARS_TO_PLAY: i32 = 8;
    // schedule playback events one second ahead of the current time
    let preroll_in_samples = samples_per_sec as u64;
    let playback_start_in_samples = player.output_sample_frame_position() + preroll_in_samples;
    for beat_counter in 0..(beats_per_bar * BARS_TO_PLAY) {
        // when is the next beat playback due?
        let next_beats_sample_time =
            (playback_start_in_samples as f64 + beat_counter as f64 * samples_per_beat) as u64;

        // play a clone of the preloaded sample at the next beat's sample time
        let playback_speed = if (beat_counter % beats_per_bar) == 0 {
            speed_from_note(60 + 12) // raise pitch by an octave every bar
        } else {
            speed_from_note(60)
        };
        let sample = if (beat_counter / beats_per_bar) % 4 < 2 {
            metronome_sample.clone(
                FilePlaybackOptions::default().speed(playback_speed),
                sample_rate,
            )? // play the cowell every 2 bars
        } else {
            bass_sample.clone(
                FilePlaybackOptions::default().speed(playback_speed),
                sample_rate,
            )? // else the bass
        };

        let playback_id = player.play_file_source(sample, Some(next_beats_sample_time))?;

        // stop (fade out) the source before the next one starts
        player.stop_source_at_sample_time(
            playback_id,
            next_beats_sample_time + samples_per_beat as u64,
        )?;

        // sleep until the next even is due
        let current_output_frame_position = player.output_sample_frame_position();
        if next_beats_sample_time > current_output_frame_position + preroll_in_samples {
            let seconds_until_next_beat = samples_to_seconds(
                next_beats_sample_time - current_output_frame_position - preroll_in_samples,
            );
            // wake up roughly 2 seconds before the next beat is due
            let seconds_to_sleep = seconds_until_next_beat - 1.0;
            if seconds_to_sleep > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(seconds_to_sleep));
            }
        }
    }

    // wait until playback finished
    let total_samples_to_play =
        (samples_per_beat * beats_per_bar as f64 * BARS_TO_PLAY as f64) as u64;
    let samples_until_playback_finished = total_samples_to_play as i64
        - player.output_sample_frame_position() as i64
        + playback_start_in_samples as i64;
    if samples_until_playback_finished > 0 {
        let seconds_until_playback_finished =
            samples_to_seconds(samples_until_playback_finished as u64);
        std::thread::sleep(Duration::from_secs_f64(
            seconds_until_playback_finished + 1.0,
        ));
    }

    Ok(())
}
