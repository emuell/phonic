**phonic** is a cross-platform audio playback and DSP library for Rust, providing a flexible, low-latency audio engine and related tools for games and music applications.

Originally developed for the [AFEC-Explorer](https://github.com/emuell/AFEC-Explorer) app, phonic initially addressed the need for precise playback position monitoring not found in other Rust audio libraries. It is now also used as the default sample playback engine for the experimental algorithmic sequencer [pattrns](https://github.com/renoise/pattrns).

> [!NOTE] 
> phonic has not yet reached a stable version, so expect breaking changes. The effects API in particular is a work in progress and will likely change in the future.


### Features

- **Cross-Platform Audio Playback**:
  - Play audio on Windows, macOS, and Linux via [cpal](https://github.com/RustAudio/cpal).
  - WebAssembly support for in-browser audio via [emscripten](https://emscripten.org/).
  - Optional WAV file output device for rendering computed audio to a file instead of playing it back.

- **Flexible Audio Source Handling**:
  - Play, seek, stop, and mix **preloaded** (buffered) or **streamed** (on-the-fly decoded) audio files.
  - Support for most common audio formats through [Symphonia](https://github.com/pdeljanov/Symphonia).
  - Automatic resampling and channel mapping via a fast custom resampler and [Rubato](https://github.com/HEnquist/rubato).
  - Seamless loop playback using loop points from WAV and FLAC files.

- **Advanced Playback Control**:
  - **Sample-precise scheduling** for accurate sequencing.
  - Real-time monitoring of playback position and status for GUI integration.
  - Dynamic control over volume, panning, and playback speed.

- **Custom Synthesis and DSPs**:
  - Build simple or complex **DSP graphs** by routing audio through optional sub-mixers.
  - Play completely custom-built synthesizers or use the optional [dasp](https://github.com/RustAudio/dasp) integration for creating synth sources.
  - Apply custom-built DSP effects or use built-in effects (reverb, chorus, filter, compressor).


### Documentation

Rust docs for the last published versions are available at <https://docs.rs/phonic>


### Examples

See [/examples](https://github.com/emuell/phonic/tree/master/examples) directory for more examples.


#### File Playback with Monitoring

Play, seek and stop audio files on the default audio output device.
Monitor playback status of playing files.

```rust no_run
use phonic::{
    DefaultOutputDevice, Player, PlaybackStatusEvent, Error, 
    FilePlaybackOptions, SynthPlaybackOptions
};

fn main() -> Result<(), Error> {
    // Open the default audio device (cpal or web, depending on the enabled output feature)
    let output_device = DefaultOutputDevice::open()?;
    // Create an optional channel to receive playback status events ("Position", "Stopped")
    // Prefer using a bounded channel here to avoid memory allocations in the audio thread.
    let (playback_status_sender, playback_status_receiver) = crossbeam_channel::bounded(32);
    // Create a player and transfer ownership of the output device to the player. The player
    // will play, mix down and manage file and synth sources for us from here.
    let mut player = Player::new(output_device, playback_status_sender);

    // Start playing a file: The file below is going to be "preloaded" because it uses the 
    // default playback options. Preloaded means it's entirely decoded first, then played back
    // from a decoded buffer. All files played through the player are automatically resampled
    // and channel-mapped to match the audio output's signal specs.
    let small_file_id = player.play_file(
        "PATH_TO/some_small_file.wav",
        FilePlaybackOptions::default())?;
    // The next file is going to be decoded and streamed on the fly, which is especially handy
    // for long files, as it can start playing right away and won't need to allocate memory 
    // for the entire file. 
    // We're also repeating the file playback 2 times, lowering the volume and are pitching
    // it down a bit. As the player mixes down everything, we'll hear both files at the same 
    // time now:
    let long_file_id = player.play_file(
        "PATH_TO/some_long_file.mp3",
        FilePlaybackOptions::default()
            .streamed()
            .volume_db(-6.0)
            .speed(0.5)
            .repeat(2),
    )?;

    // You can optionally track playback status events from the player:
    std::thread::spawn(move || {
        while let Ok(event) = playback_status_receiver.recv() {
            match event {
                PlaybackStatusEvent::Position { id, path, context: _, position } => {
                    // context is an optional, user defined payload,
                    // passed along with `player.play_file_with_context` 
                    println!("Playback pos of source #{id} '{path}': {pos}",
                        pos = position.as_secs_f32()
                    );
                }
                PlaybackStatusEvent::Stopped { id, path, context: _, exhausted, } => {
                    if exhausted {
                        println!("Playback of #{id} '{path}' finished");
                    } else {
                        println!("Playback of #{id} '{path}' was stopped");
                    }
                }
            }
        }
    });

    // Playing files can be manipulated in various ways. Here we seek and stop a file:
    player.seek_source(long_file_id, std::time::Duration::from_secs(5), None)?;
    player.stop_source(small_file_id, None)?;

    // If you only want one file to play at the same time, simply stop all playing
    // sounds before starting a new one:
    player.stop_all_sources()?;
    player.play_file("PATH_TO/boom.wav", FilePlaybackOptions::default())?;

    Ok(())
}
```

#### File playback with DSP Effects in a Mixer Graph

Create complex audio processing chains by routing sources through different mixers and effects.

```rust no_run
use phonic::{
    DefaultOutputDevice, Player, Error, FilePlaybackOptions, 
    effects::{ChorusEffect, ReverbEffect}
};

fn main() -> Result<(), Error> {
    // Create a player with the default output device
    let mut player = Player::new(DefaultOutputDevice::open()?, None);

    // Add a reverb effect to the main mixer. All sounds played without a
    // specific target mixer will now be routed through this effect.
    player.add_effect(ReverbEffect::with_parameters(0.6, 0.8), None)?;

    // Create a new sub-mixer that is a child of the main mixer.
    let chorus_mixer_id = player.add_mixer(None)?;
    // Add a chorus effect to this new mixer. Sources routed to this mixer will
    // now apply the chorus effect and reverb (the main mixer effects) 
    player.add_effect(ChorusEffect::default(), chorus_mixer_id)?;

    // Play a file through the main mixer (which has reverb only).
    player.play_file(
        "PATH_TO/some_file.wav",
        FilePlaybackOptions::default(),
    )?;

    // Play another file through the chorus mixer (and main mixer with the reverb FX).
    player.play_file(
        "PATH_TO/another_file.wav",
        FilePlaybackOptions::default().target_mixer(chorus_mixer_id),
    )?;

    Ok(())
}
```

## Contributing

Patches are welcome! Please fork the latest git repository and create a feature or bugfix branch.


## License

**phonic** is distributed under the terms of the [GNU Affero General Public License V3](https://www.gnu.org/licenses/agpl-3.0.html).
