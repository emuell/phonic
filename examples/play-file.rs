use afstream::player::{
    file::AudioPlayerFile,
    output::{AudioOutput, DefaultAudioOutput},
    PlaybackEvent, PlaybackManager,
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;

    // create playback manager
    let (event_sx, event_rx) = crossbeam_channel::unbounded();
    let mut playback_manager = PlaybackManager::new(audio_output.sink(), event_sx);

    // load sound from given file path
    let source = AudioPlayerFile::new("assets/altijd synth bit.wav".to_string())
        .map_err(|err| err.to_string())?;

    // play the file
    playback_manager.play(source);

    // handle events from playback manager
    let event_thread = std::thread::spawn(move || loop {
        match event_rx.recv() {
            Ok(event) => match event {
                PlaybackEvent::Position { path: _, position } => {
                    println!("Playback pos: {}", position.as_secs_f32());
                }
                PlaybackEvent::EndOfFile { path: _ } => {
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
