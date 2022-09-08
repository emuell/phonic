
## afplay

**afplay** is a cross-platform *audio playback library for Rust*, based on jpochyla's [psst-core](https://github.com/jpochyla/psst/tree/master/psst-core) audio playback implementation.

It aims to be a suitable player for game engines, but can also be used as a general-purpose playback engine for other types of music applications.<br>
It borrows a few ideas from [rodio](https://github.com/RustAudio/rodio), such as the AudioSource trait, but also tries to fill the gaps that rodio never filled.  

It was originally developed and is used in the [AFEC-Explorer](https://github.com/emuell/AFEC-Explorer) app and related projects which are using the excellent [Tauri](https://tauri.app) app framework.

### Features

- Play, seek, stop, mix and monitor playback of preloaded or on-the-fly decoded (streamed) *audio files*.
- Play, stop, mix and monitor playback of custom *synth tones* thanks to [dasp](https://github.com/RustAudio/dasp) (can be optionally enabled).
- Audio output for Windows, macOS and Linux is handled via [cpal](https://github.com/RustAudio/cpal) or [cubeb](https://github.com/mozilla/cubeb).
- Decodes and thus plays back *most common audio file formats*, thanks to [Symphonia](https://github.com/pdeljanov/Symphonia).
- Files are *automatically resampled* and *channel mapped* to the audio output's signal specs, thanks to [libsamplerate](https://github.com/RamiHg/rust-libsamplerate).

### Examples

See [example directory](./examples) for some more working examples. 

```rust
use afplay::{playback::PlaybackStatusEvent, AudioFilePlayer, AudioOutput, DefaultAudioOutput};
use dasp::{Frame, Signal};

// Open default device (cpal or cubeb, whatever is enabled as audio output feature)
let audio_output = DefaultAudioOutput::open()?;
let audio_sink = audio_output.sink();

// Create an optional channel to receive playback status events (Position, Stopped events)
let (playback_status_sender, playback_status_receiver) = crossbeam_channel::unbounded();
// Create a player which we'll use to play, mix and manage file or synth sources.
let mut player = AudioFilePlayer::new(audio_sink, Some(playback_status_sender));

// Create sound sources and memorize their ids for the playback status and control.
// The first file is preloaded - which means its entirely decoded first, then played back buffered:
let small_file_id = player.play_preloaded_file("PATH_TO/some_small_file.wav")?;
// The second file is going to be decoded and streamed on the fly, which is handy for large files.
// The player mixes all added files, so we'll hear both files at once later:
let large_file_id = player.play_streamed_file("PATH_TO/some_long_file.mp3")?;

// We're playing a sinple synth tone as well. You can pass any dasp::signal::Signal here. 
// It will be wrapped in a dasp::signal::UntilExhausted, so it can be used for one-shots to.
// NB: The optional `dasp-synth` feature needs to be enabled in afplay for this to work! 
let dasp_signal = dasp::signal::rate(audio_sample_rate as f64).const_hz(440.0).sine();
let synth_id = player.play_dasp_synth(some_dasp_signal, "my_synth_sound".to_string())?;

// Somewhere in your code you can optionally handle playback status events from the player:
std::thread::spawn(move || {
    while let Ok(event) = playback_status_receiver.recv() {
        match event {
            PlaybackStatusEvent::Position { id, path, position } => {
                println!(
                    "Playback pos of source #{} '{}': {}",
                    id,
                    path,
                    position.as_secs_f32()
                );
            }
            PlaybackStatusEvent::Stopped {
                id,
                path,
                exhausted,
            } => {
                if exhausted {
                    println!("Playback of #{} '{}' finished", id, path);
                } else {
                    println!("Playback of #{} '{}' was stopped", id, path);
                }
            }
        }
    }
});

// All playing *file* sources can be seeked or stopped:
player.seek_source(large_file_id, std::time::Duration::from_secs(5))?;
player.stop_source(small_file_id)?;

// New files can be started any time. Tthey will be mixed together with whatever 
// else is currently playing.
let _another_file_id = player.play_preloaded_file("PATH_TO/bang.wav")?;

// Finally: stop and drop all playing sources
player.stop_all_sources()?;
// Or simply drop the player or audio_device to stop and dealloc everything:
drop(player);
```

## License

afplay is distributed under the terms of both the MIT license and the Apache License (Version 2.0).

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
