use afstream::{
    output::{AudioOutput, DefaultAudioOutput},
    player::AudioFilePlayer,
    source::decoded::DecoderPlaybackEvent,
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;

    // create playback manager
    let (event_sx, event_rx) = crossbeam_channel::unbounded();
    let mut player = AudioFilePlayer::new(audio_output.sink(), event_sx);

    // load sound from given file path and stream it
    player
        .play("assets/altijd synth bit.wav".to_string())
        .map_err(|err| err.to_string())?;

    // handle player events
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
