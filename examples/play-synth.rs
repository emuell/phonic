use dasp::{signal, Frame, Signal};

use afplay::{
    synth::SynthPlaybackStatusMsg,
    AudioFilePlayer, {AudioOutput, AudioSink, DefaultAudioOutput},
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;
    let audio_sink = audio_output.sink();
    let sample_rate = audio_sink.sample_rate();

    // create channel for playback status events
    let (event_sx, event_rx) = crossbeam_channel::unbounded();
    // create a source player
    let file_event_sx = None;
    let mut player = AudioFilePlayer::new(audio_sink, file_event_sx, Some(event_sx));

    // Creates a signal of a detuned sine.
    let generate_note = |pitch: f64, amplitude: f64, duration: u32| {
        let fundamental = signal::rate(sample_rate as f64).const_hz(pitch);
        let harmonic_l1 = signal::rate(sample_rate as f64).const_hz(pitch * 2.01);
        let harmonic_h1 = signal::rate(sample_rate as f64).const_hz(pitch / 2.02);
        let harmonic_h2 = signal::rate(sample_rate as f64).const_hz(pitch / 4.04);

        signal::from_iter(
            fundamental
                .sine()
                .add_amp(harmonic_l1.sine().scale_amp(0.5))
                .add_amp(harmonic_h1.sine().scale_amp(0.5))
                .add_amp(harmonic_h2.sine().scale_amp(0.5))
                .scale_amp(amplitude)
                .take(duration as usize)
                .zip(0..duration)
                .map(move |(s, index)| {
                    let env: f64 = (1.0 - (index as f64) / (duration as f64)).powf(2.0);
                    (s * env).to_float_frame()
                }),
        )
    };

    // combine 3 notes to a chord
    let note_amp = 0.5_f64;
    let note_duration = 4 * sample_rate;

    let chord = // chord
        generate_note(440_f64, note_amp, note_duration)
        .add_amp(generate_note(
            440_f64 * (4.0 / 3.0),
            note_amp,
            note_duration,
        ))
        .add_amp(generate_note(
            440_f64 * (6.0 / 3.0),
            note_amp,
            note_duration,
        ));

    // create audio source for the chord and memorize the id for the playback status
    let mut playing_synth_ids = vec![player
        .play_dasp_synth(chord)
        .map_err(|err| err.to_string())?];

    // handle events from the file sources
    let event_thread = std::thread::spawn(move || loop {
        match event_rx.recv() {
            Ok(event) => match event {
                SynthPlaybackStatusMsg::Exhausted { synth_id } => {
                    println!("Playback of synth #{} finished", synth_id);
                    playing_synth_ids.retain(|v| *v != synth_id);
                    if playing_synth_ids.is_empty() {
                        // stop thread when all synths finished
                        break;
                    }
                }
            },
            Err(err) => {
                log::info!("Playback event channel closed: '{err}'");
                break;
            }
        }
    });

    // wait until playback finished
    event_thread.join().unwrap();

    Ok(())
}
