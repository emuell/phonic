phonic
======

[![crates.io](https://img.shields.io/crates/v/phonic.svg)](https://crates.io/crates/phonic)
[![docs.rs](https://docs.rs/phonic/badge.svg)](https://docs.rs/phonic)
[![license](https://img.shields.io/crates/l/phonic.svg)](https://choosealicense.com/licenses/agpl-3.0/)

phonic is a cross-platform audio playback and DSP library for Rust. It provides a flexible, low-latency audio engine and DSP tools for desktop and web-based music application development.

Originally developed for the [AFEC-Explorer](https://github.com/emuell/AFEC-Explorer) app, phonic was created to provide precise playback position monitoring - a feature lacking in other Rust audio libraries at the time. It is now also used in the experimental algorithmic sequencer [pattrns](https://github.com/renoise/pattrns) as the default sample playback engine.

> [!NOTE]
> phonic has not yet reached a stable version, so expect breaking changes.

### Features

- **Cross-Platform Audio Playback**:
  - Play audio on Windows, macOS, and Linux via [cpal](https://github.com/RustAudio/cpal).
  - WebAssembly support for in-browser audio via [emscripten](https://emscripten.org/).
  - WAV file output for rendering computed audio to a file instead of playing it back.

- **Flexible Audio Source Handling**:
  - Play, seek, stop, and mix **preloaded** (buffered) or **streamed** (on-the-fly decoded) audio files.
  - Support for most common audio formats through [Symphonia](https://github.com/pdeljanov/Symphonia).
  - Seamless loop playback using loop points from WAV and FLAC files.
  - Automatic resampling and channel mapping via a fast custom resampler and [Rubato](https://github.com/HEnquist/rubato).

- **Advanced Playback Control**:
  - Sample-precise **scheduling** of playback events for accurate sequencing.
  - Real-time **monitoring** of playback position and status for GUI integration.
  - Dynamic control over volume, panning, and playback speed via `Sync` + `Send` playback handles.

- **Custom Synthesis and DSPs**:
  - Play preloaded sample files via a basic **sampler**.
  - Play custom-built **synthesizers** or one-shot synth sounds using the optional [fundsp](https://github.com/SamiPerttu/fundsp) feature.
  - Apply custom-built or use built-in **DSP effects**: gain, filter, eq, reverb, chorus, compressor, limiter, distortion.
  - Build simple or complex **DSP graphs** by routing audio through optional sub-mixers.
  - DSP effects are automatically bypassed to save CPU cycles when they receive no audible input.

- **Performance Monitoring**:
  - Measure the CPU load of the main mixer and individual audio sources in real-time.
  - Access average and peak load metrics to identify performance bottlenecks.

### Documentation

Rust docs for the last published versions are available at <https://docs.rs/phonic>

### Examples

See [/examples](https://github.com/emuell/phonic/tree/master/examples) directory for more examples.

#### File Playback with Monitoring

Play, seek, and stop audio files on the default audio output device.
Monitor playback status of playing files.

```rust no_run
use std::{time::Duration, sync::mpsc::sync_channel};

use phonic::{
    DefaultOutputDevice, Player, PlaybackStatusEvent, Error,
    FilePlaybackOptions, SynthPlaybackOptions
};

fn main() -> Result<(), Error> {
    // Create a player with the default output device and a channel to receive playback events.
    let (playback_status_sender, playback_status_receiver) = sync_channel(32);
    let mut player = Player::new(DefaultOutputDevice::open()?, playback_status_sender);

    // Start playing a file: The file below is going to be "preloaded" because it uses the
    // default playback options. Preloaded means it's entirely decoded first, then played back
    // from a decoded buffer. All files played through the player are automatically resampled
    // and channel-mapped to match the audio output's signal specs.
    // Preloaded files can also be cheaply cloned, so they can be allocated once and played back
    // many times too. The returned handle allows changing playback properties of the files.
    let small_file = player.play_file(
        "PATH_TO/some_small_file.wav",
        FilePlaybackOptions::default())?;

    // The next file is going to be decoded and streamed on the fly, which is especially handy
    // for long files, as it can start playing right away and won't need to allocate memory
    // for the entire file.
    let long_file = player.play_file(
        "PATH_TO/some_long_file.mp3",
        FilePlaybackOptions::default()
            .streamed()
            .volume_db(-6.0)
            .speed(0.5)
            .repeat(2),
    )?;

    // You can optionally track playback status events from the player.
    std::thread::spawn(move || {
        while let Ok(event) = playback_status_receiver.recv() {
            match event {
                PlaybackStatusEvent::Position { id, path, context: _, position } => {
                    // `context` is an optional, user-defined payload, which can be passed
                    // along to the status with `player.play_file_with_context`
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

    // The returned handles allow controlling playback properties of playing files.
    // The second arg is an optional sample time, where `None` means immediately.
    long_file.seek(Duration::from_secs(5), None)?;

    // Using Some sample time args, we can schedule changes (sample-accurate).
    let now = player.output_sample_frame_position();
    let samples_per_second = player.output_sample_rate() as u64;

    // Use the handle's `is_playing` function to check if a file is still playing.
    if long_file.is_playing() {
        long_file.set_volume(0.3, now + samples_per_second)?; // Fade down after 1 second
        long_file.stop(now + 2 * samples_per_second)?;  // Stop after 2 seconds
    }

    // If you only want one file to play at the same time, stop all playing sounds.
    player.stop_all_sources()?;

    // And then schedule a new source for playback.
    let _boom = player.play_file("PATH_TO/boom.wav", FilePlaybackOptions::default())?;

    Ok(())
}
```

#### File Playback with Generators, DSP Effects in a Mixer Graph

Create DSP graphs by routing sources through different mixers and effects.

```rust no_run
use std::time::Duration;

use phonic::{
    DefaultOutputDevice, Player, Error, FilePlaybackOptions,
    effects::{ChorusEffect, ReverbEffect}, ParameterValueUpdate, FourCC,
    generators::Sampler, GeneratorPlaybackOptions,
};

fn main() -> Result<(), Error> {
    // Create a player with the default output device.
    let mut player = Player::new(DefaultOutputDevice::open()?, None);

    // Add a reverb effect to the main mixer. All sounds played without a
    // specific target mixer will now be routed through this effect.
    let reverb = player.add_effect(ReverbEffect::with_parameters(0.6, 0.8), None)?;

    // Create a new sub-mixer that is a child of the main mixer.
    let chorus_mixer = player.add_mixer(None)?;
    // Add a chorus effect to this new mixer. Sources routed to this mixer will
    // now apply the chorus effect and reverb (the main mixer effects).
    let chorus = player.add_effect(ChorusEffect::default(), chorus_mixer.id())?;

    // Effect parameters can be automated via the returned handles.
    // The `None` arguments are optional sample times to schedule events.
    reverb.set_parameter(ReverbEffect::ROOM_SIZE.value_update(0.9), None)?;
    // Or if no parameter description is available (e.g. in UIs), send a normalized value. 
    chorus.set_parameter((FourCC(*b"rate"), ParameterValueUpdate::Normalized(0.5)), None)?;

    // Play a file through the main mixer (which has reverb only).
    let some_file = player.play_file(
        "PATH_TO/some_file.wav",
        FilePlaybackOptions::default(),
    )?;
    // Play another file through the chorus mixer (and main mixer with the reverb FX).
    let another_file = player.play_file(
        "PATH_TO/another_file.wav",
        FilePlaybackOptions::default().target_mixer(chorus_mixer.id()),
    )?;

    // Create a sampler generator to play a sample.
    // We configure it to play on the chorus mixer.
    let generator = player.play_generator(
        Sampler::from_file(
            "path/to/some_sample.wav",
            None,
            GeneratorPlaybackOptions::default().target_mixer(sub_mixer.id())
            player.output_channel_count(),
            player.output_sample_rate(),
        )?,
        None
    )?;
    
    // Trigger a note on the generator. The `generator` handle is `Send + Sync`, so
    // you could also pass it to some other thread (e.g. a MIDI thread) to trigger
    // the generator from there.
    generator.note_on(60, Some(1.0f32), None, None)?;

    // [... do something else ...]
 
    // Stop generator and sample files (keep `some_file` running until it plays to the end)
    another_file.stop(None)?;
    generator.stop(None)?;

    // Wait until all files and generators finished playing
    while some_file.is_playing() || another_file.is_playing() || generator.is_playing() {
        std::thread::sleep(Duration::from_millis(500));
    }

    Ok(())
}
```

## Contributing

Patches are welcome! Please fork the latest git repository and create a feature or bugfix branch.

## License

phonic is distributed under the terms of the [GNU Affero General Public License V3](https://www.gnu.org/licenses/agpl-3.0.html).
