use afstream::{
    output::{AudioOutput, DefaultAudioOutput},
    player::AudioFilePlayer,
    source::decoded::DecoderPlaybackEvent,
};

fn main() -> Result<(), String> {
    // Open default device
    let audio_output = DefaultAudioOutput::open().map_err(|err| err.to_string())?;
    let audio_sink = audio_output.sink();

    // create channel for decoder source events
    let (event_sx, event_rx) = crossbeam_channel::unbounded();
    let mut player = AudioFilePlayer::new(audio_sink, event_sx);

    // create sound sources and memorize file ids
    let mut file_ids = vec![
        player
            .play_file("assets/altijd synth bit.wav".to_string())
            .map_err(|err| err.to_string())?,
        player
            .play_file("assets/BSQ_M14.wav".to_string())
            .map_err(|err| err.to_string())?,
    ];

    // start playing
    player.start();

    // handle events from the decoder sources
    let event_thread = std::thread::spawn(move || loop {
        match event_rx.recv() {
            Ok(event) => match event {
                DecoderPlaybackEvent::Position {
                    file_id,
                    file_path: path,
                    position,
                } => {
                    println!(
                        "Playback pos of file #{} '{}': {}",
                        file_id,
                        path,
                        position.as_secs_f32()
                    );
                }
                DecoderPlaybackEvent::EndOfFile {
                    file_id,
                    file_path: path,
                } => {
                    println!("Playback of #{} '{}' finished", file_id, path);
                    file_ids.retain(|v| *v != file_id);
                    if file_ids.is_empty() {
                        // stop thread when all files finished
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
