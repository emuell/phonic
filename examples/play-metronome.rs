use phonic::{
    utils::speed_from_note, DefaultOutputDevice, Error, FilePlaybackOptions, OutputDevice, Player,
    PreloadedFileSource,
};

// -------------------------------------------------------------------------------------------------

#[cfg(all(debug_assertions, feature = "assert_no_alloc"))]
#[global_allocator]
static A: assert_no_alloc::AllocDisabler = assert_no_alloc::AllocDisabler;

// -------------------------------------------------------------------------------------------------

fn main() -> Result<(), Error> {
    // Setup audio output and player
    let mut player = Player::new(DefaultOutputDevice::open()?.sink(), None);
    let sample_rate = player.output_sample_rate();

    // Preload samples
    let cowbell = PreloadedFileSource::from_file(
        "assets/cowbell.wav",
        None,
        Default::default(),
        sample_rate,
    )?;
    let bass =
        PreloadedFileSource::from_file("assets/bass.wav", None, Default::default(), sample_rate)?;

    // Metronome parameters
    const BPM: f64 = 120.0;
    const BEATS_PER_BAR: usize = 4;
    const BARS_TO_PLAY: usize = 8;

    let samples_per_beat = (60.0 / BPM * sample_rate as f64) as u64;
    let output_start_time = player.output_sample_frame_position() + sample_rate as u64; // Start 1 second ahead

    // Schedule all beats
    for beat in 0..(BEATS_PER_BAR * BARS_TO_PLAY) {
        // When is the next beat due?
        let sample_time = output_start_time + beat as u64 * samples_per_beat;
        // Alternate between cowbell and bass every 2 bars
        let sample = if (beat / (2 * BEATS_PER_BAR)) % 2 == 0 {
            &cowbell
        } else {
            &bass
        };
        // Raise pitch by octave on first beat of each bar
        let speed = speed_from_note(60 + if beat % BEATS_PER_BAR == 0 { 12 } else { 0 });
        // Play sample at current beat
        let playback_id = player.play_file_source(
            sample.clone(FilePlaybackOptions::default().speed(speed), sample_rate)?,
            Some(sample_time),
        )?;
        // Stop sample at the next beat
        player.stop_source_at_sample_time(playback_id, sample_time + samples_per_beat)?;
    }

    // Wait for playback to finish
    let duration_samples = (BEATS_PER_BAR * BARS_TO_PLAY) as u64 * samples_per_beat;
    while player.output_sample_frame_position() < output_start_time + duration_samples {
        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    Ok(())
}
