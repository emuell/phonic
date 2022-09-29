use afplay::{
    utils::speed_from_note, AudioFilePlaybackStatusEvent, AudioFilePlayer, AudioOutput,
    DefaultAudioOutput, Error, FilePlaybackOptions,
};

fn main() -> Result<(), Error> {
    // Open default device
    let audio_output = DefaultAudioOutput::open()?;

    // create channel for playback status events
    let (status_sender, status_receiver) = crossbeam_channel::unbounded();
    let mut player = AudioFilePlayer::new(audio_output.sink(), Some(status_sender));

    // pause playing until we've added all sources
    player.stop();

    // create sound sources and memorize file ids for the playback status
    let mut playing_file_ids = vec![
        player
            // files are by default not streamed but are preloaded and player buffered.
            .play_file(
                "assets/altijd synth bit.wav",
                FilePlaybackOptions::default(),
            )?,
        player
            // this file is going to be streamed on the fly with a lowered volume.
            // we're also lowering the volume and do loop the file 2 times
            .play_file(
                "assets/YuaiLoop.wav",
                FilePlaybackOptions::default()
                    .streamed()
                    .volume_db(-3.0)
                    .speed(speed_from_note(58))
                    .repeat(1),
            )?,
    ];

    // start playing
    player.start();

    // handle events from the file sources
    let event_thread = std::thread::spawn(move || {
        while let Ok(event) = status_receiver.recv() {
            match event {
                AudioFilePlaybackStatusEvent::Position { id, path, position } => {
                    println!(
                        "Playback pos of file #{} '{}': {}",
                        id,
                        path,
                        position.as_secs_f32()
                    );
                }
                AudioFilePlaybackStatusEvent::Stopped {
                    id,
                    path,
                    exhausted,
                } => {
                    if exhausted {
                        println!("Playback of #{} '{}' finished playback", id, path);
                    } else {
                        println!("Playback of #{} '{}' was stopped", id, path);
                    }
                    playing_file_ids.retain(|v| *v != id);
                    if playing_file_ids.is_empty() {
                        // stop thread when all synths finished
                        break;
                    }
                }
            }
        }
    });

    // wait until playback of all files finished
    event_thread.join().unwrap();

    Ok(())
}
