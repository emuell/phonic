use afstream::{
    player::output::{AudioOutput, AudioSink, DefaultAudioOutput},
    source::decoded::{DecoderPlaybackEvent, DecoderSource},
    source::mixed::MixedSource,
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;
    let audio_sink = audio_output.sink();

    // load sound from given file path
    let (event_sx, event_rx) = crossbeam_channel::unbounded();
    let synth_source =
        DecoderSource::new("assets/altijd synth bit.wav".to_string(), event_sx.clone())
            .map_err(|err| err.to_string())?;
    let loop_source = DecoderSource::new("assets/BSQ_M14.wav".to_string(), event_sx.clone())
        .map_err(|err| err.to_string())?;

    let mixer = MixedSource::new();
    mixer.add(Box::new(synth_source));
    mixer.add(Box::new(loop_source));

    // play the file
    audio_sink.play(mixer);
    audio_sink.resume();

    // handle events from playback manager
    let event_thread = std::thread::spawn(move || loop {
        match event_rx.recv() {
            Ok(event) => match event {
                DecoderPlaybackEvent::Position { path: _, position } => {
                    println!("Playback pos: {}", position.as_secs_f32());
                }
                DecoderPlaybackEvent::EndOfFile { path: _ } => {
                    println!("Playback finished");
                    break;
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
