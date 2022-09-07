
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
use dasp::{Frame, Signal};
use afplay::{file::FilePlaybackStatusMsg, synth::SynthPlaybackStatusMsg, *};

// Open default device (cpal or cubeb, whatever is enabled as audio output feature)
let audio_output = DefaultAudioOutput::open()?;
let audio_sink = audio_output.sink();

// Create an optional channel for file playback status events (Position, EndOfFile events)
let (file_event_sx, file_event_rx) = crossbeam_channel::unbounded();
// Create an optional channel for synth status events (Exhausted events)
let (synth_event_sx, synth_event_rx) = crossbeam_channel::unbounded();

// Create a player which we'll use to play, mix and manage file or synth sources.
let mut player = AudioFilePlayer::new(audio_sink, Some(file_event_sx), Some(synth_event_sx));

// Create sound sources and memorize their ids for the playback status and control.
// The first file is preloaded - which means its entirely decoded first, then played back buffered:
let some_small_file_id = player.play_preloaded_file(
    "some_small_file.wav".to_string())?;
// The second file is going to be decoded and streamed on the fly, which is handy for large files.
// The player mixes all added files, so we'll hear both files at once later:
let some_large_file_id = player.play_streamed_file(
    "some_really_really_large_/BSQ_M1file_.mp3".to_string())?;

// We're playing a sinple synth tone as well. You can pass any dasp::signal::Signal here. 
// It will be wrapped in a dasp::signal::UntilExhausted, so it can be used for one-shots to.
let some_dasp_signal = dasp::signal::rate(audio_sample_rate as f64).const_hz(440.0).sine();
let some_synth_id = player.play_dasp_synth(some_dasp_signal)?;

// Somewhere in your code, likely in a background thread, handle playback status events from the player:
std::thread::spawn(move || loop {
    crossbeam_channel::select! {
        recv(file_event_rx) -> msg => {
            if let Ok(file_event) = msg {
                match file_event {
                    FilePlaybackStatusMsg::Position { file_id, file_path, position } => {
                        println!("Playback pos of file #{} '{}': {}", 
                            file_id, file_path, position.as_secs_f32());
                    },
                    FilePlaybackStatusMsg::Stopped { file_id, file_path, end_of_file } => {
                        if end_of_file {
                            println!("Playback of #{} '{}' finished playback", file_id, file_path);

                        } else {
                            println!("Playback of #{} '{}' was stopped", file_id, file_path);
                        }
                    }
                }
            }
        },
        recv(synth_event_rx) -> msg => {
            if let Ok(synth_event) = msg {
                match synth_event {
                    SynthPlaybackStatusMsg::Stopped { synth_id, exhausted } => {
                        if exhausted {
                            println!("Playback of synth #{} finished playback", synth_id);
                        } else {
                            println!("Playback of synth #{} was stopped", synth_id);
                        }
                    }
                }
            }
        }
    }
});

// All playing file sources can be seeked or stopped:
player.seek_file(some_large_file_id, std::time::Duration::from_secs(5))?;
player.stop_file(some_small_file_id)?;

// New files can be started any time. as before they will be mixed together with whatever 
// else is currently playing.
let _some_new_file_id = player.play_preloaded_file("bang.wav".to_string())?;

// Finally: stop and drop all playing sources
player.stop_all_sources()?;
// Or simply drop the player or audio_device to stop and dealloc everything:
drop(player);
```

## License

afplay is distributed under the terms of both the MIT license and the Apache License (Version 2.0).

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or http://www.apache.org/licenses/LICENSE-2.0)
* MIT license ([LICENSE-MIT](LICENSE-MIT) or http://opensource.org/licenses/MIT)
