use afstream::{
    player::output::{AudioOutput, AudioSink, DefaultAudioOutput},
    source::decoded::{DecoderPlaybackEvent, DecoderSource},
    source::mixed::MixedSource,
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;
    let audio_sink = audio_output.sink();

    // create channel for decoder source events
    let (event_sx, event_rx) = crossbeam_channel::unbounded();

    // create sound sources
    let synth_source = DecoderSource::new(
        "assets/altijd synth bit.wav".to_string(),
        Some(event_sx.clone()),
    )
    .map_err(|err| err.to_string())?;
    let loop_source = DecoderSource::new("assets/BSQ_M14.wav".to_string(), Some(event_sx))
        .map_err(|err| err.to_string())?;

    // create mixer to add sources together
    let mixer = MixedSource::new(audio_sink.channel_count(), audio_sink.sample_rate());
    mixer.add(synth_source);
    mixer.add(loop_source);

    // play the mixer source
    audio_sink.play(mixer);
    audio_sink.resume();

    // handle events from the decoder sources
    let event_thread = std::thread::spawn(move || loop {
        match event_rx.recv() {
            Ok(event) => match event {
                DecoderPlaybackEvent::Position { path, position } => {
                    println!("Playback pos of '{}': {}", path, position.as_secs_f32());
                }
                DecoderPlaybackEvent::EndOfFile { path } => {
                    println!("Playback of '{}' finished", path);
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
