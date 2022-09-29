use std::time::Duration;

use afplay::{
    source::file::preloaded::PreloadedFileSource, utils::speed_from_note, AudioFilePlayer,
    AudioOutput, DefaultAudioOutput, Error, FilePlaybackOptions,
};

fn main() -> Result<(), Error> {
    // create a player
    let mut player = AudioFilePlayer::new(DefaultAudioOutput::open()?.sink(), None);

    // preload our metronome sample
    let metronome_sample =
        PreloadedFileSource::new("assets/cowbell.wav", None, FilePlaybackOptions::default())?;

    // define our metronome speed and signature
    let beats_per_min = 120.0;
    let beats_per_bar = 4;
    let samples_per_sec = player.output_sample_rate();
    let samples_per_beat = || -> f64 { samples_per_sec as f64 * 60.0 / beats_per_min as f64 };
    let samples_to_seconds = |samples: u64| -> f64 { samples as f64 / samples_per_sec as f64 };

    // play 8 bars in this example, starting at the player's current playback pos
    let playback_start_in_samples = player.output_sample_frame_position();
    for beat_counter in 0..(beats_per_bar * 8) {
        // when is the next beat playback due?
        let next_beats_sample_time =
            (playback_start_in_samples as f64 + beat_counter as f64 * samples_per_beat()) as u64;

        // play a clone of the preloaded sample at the next beat's sample time
        let playback_speed = if (beat_counter % beats_per_bar) == 0 {
            speed_from_note(60 + 12) // raise pitch by an octave every bar
        } else {
            speed_from_note(60)
        };
        player.play_file_source(
            metronome_sample.clone(),
            playback_speed,
            Some(next_beats_sample_time),
        )?;

        // sleep until the next even is due
        if next_beats_sample_time > player.output_sample_frame_position() {
            let seconds_until_next_beat =
                samples_to_seconds(next_beats_sample_time - player.output_sample_frame_position());
            // wake up roughly 1 second before the next beat is due
            let seconds_to_sleep = seconds_until_next_beat - 1.0;
            if seconds_to_sleep > 0.0 {
                std::thread::sleep(Duration::from_secs_f64(seconds_to_sleep));
            }
        }
    }

    Ok(())
}
