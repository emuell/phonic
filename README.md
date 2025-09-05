**phonic** is an *audio playback library for Rust*, based on the
[psst-core](https://github.com/jpochyla/psst/tree/master/psst-core) audio playback
implementation. 

It serves as a general-purpose, low-latency audio playback engine for Rust-based music applications while also being suitable as an audio backend for game engines.

Originally developed for the [AFEC-Explorer](https://github.com/emuell/AFEC-Explorer) app, phonic addressed the need for precise playback position monitoring not found in other Rust audio libraries. It is also used as the default sample playback engine for the experimental algorithmic sequencer [pattrns](https://github.com/renoise/pattrns).

### Features

- Play, seek, stop, mix and monitor playback of **preloaded** (buffered) or **streamed**
  (on-the-fly decoded) **audio files**.
- Play, stop, mix and monitor playback of **custom synth tones** thanks to
  [dasp](https://github.com/RustAudio/dasp) (optional feature: disabled by default).
- Build complex **DSP graphs** by routing audio through sub-mixers and applying built-in or custom **effects** like reverb, chorus, and filters.
- Play audio on Windows, macOS, Linux or the Web via [cpal](https://github.com/RustAudio/cpal) or
  [sokol-audio](https://github.com/floooh/sokol-rust) (cpal is enabled by default).
- Decodes and thus plays back most **common audio file formats**, thanks to
  [Symphonia](https://github.com/pdeljanov/Symphonia).
- Reads and applies loop points of WAV and FLAC files for **seamless loop playback**. 
- Files are automatically **resampled and channel mapped** using a fast custom cubic resampler or [rubato](https://github.com/HEnquist/rubato).
- Runs on the **web** via [sokol](https://github.com/floooh/sokol-rust) thanks to [emscripten](https://emscripten.org/): see [play-emscripten](./examples/play-emscripten/) example.
- When stopping sounds, by default a very short volume fade-out is applied to **avoid clicks**.
- Sample pitch can be gradually changed, to apply **portamento** alike note gliding in samples. 
- Allows sample precise playback scheduling, e.g. to play back sounds in a **sequencer**.
- Monitor **playback positions** and status of all played back files in e.g. GUIs. 

### Examples

See [/examples](https://github.com/emuell/phonic/tree/master/examples) directory for more examples.

#### Simple Playback

Play and stop audio files on the system's default audio output device.

```rust no_run
use phonic::{
    Player, OutputDevice, OutputSink, DefaultOutputDevice, Error, FilePlaybackOptions
};

fn main() -> Result<(), Error> {
    // Open the default audio device (cpal or sokol, depending on the enabled output feature)
    let device = DefaultOutputDevice::open()?;
    // Create a player and transfer ownership of the audio output to the player.
    let mut player = Player::new(device.sink(), None);

    // Play back a file with the default playback options.
    player.play_file(
        "PATH_TO/some_file.wav",
        FilePlaybackOptions::default())?;
    // Play back another file on top with custom playback options.
    player.play_file(
        "PATH_TO/some_long_file.mp3",
        FilePlaybackOptions::default()
            .streamed() // decode the file on-the-fly
            .volume_db(-6.0) // lower the volume a bit
            .speed(0.5) // play file at half the speed
            .repeat(2), // repeat, loop it 2 times
    )?;

    // Stop all playing files: this will quickly fade-out all playing files to avoid clicks.
    player.stop_all_sources()?;

    Ok(())
}
```

#### Advanced Playback

Play, seek and stop audio files on the default audio output device.
Monitor playback status of playing files.

```rust no_run
use phonic::{
    Player, OutputDevice, OutputSink, PlaybackStatusEvent, 
    DefaultOutputDevice, Error, FilePlaybackOptions, SynthPlaybackOptions 
};

fn main() -> Result<(), Error> {
    // Open the default audio device (cpal or sokol, depending on the enabled output feature)
    let device = DefaultOutputDevice::open()?;

    // Create an optional channel to receive playback status events ("Position", "Stopped")
    // Prefer using a bounded channel here to avoid memory allocations in the audio thread.
    let (playback_status_sender, playback_status_receiver) = crossbeam_channel::bounded(32);
    // Create a player and transfer ownership of the audio output to the player. The player
    // will play, mix down and manage all files and synth sources for us from here.
    let mut player = Player::new(device.sink(), Some(playback_status_sender));

    // We'll start playing a file now: The file below is going to be "preloaded" because it
    // uses the default playback options. Preloaded means it's entirely decoded first, then 
    // played back from a buffer.
    // Files played through the player are automatically resampled and channel-mapped to match
    // the audio output's signal specs, so there's nothing more to do to get it played:
    let small_file_id = player.play_file(
        "PATH_TO/some_small_file.wav",
        FilePlaybackOptions::default())?;
    // The next file is going to be decoded and streamed on the fly, which is especially handy
    // for long files such as music, as it can start playing right away and won't need to
    // allocate memory for the entire file.
    // We're also repeating the file playback 2 times, lowering the volume and are pitching it
    // down. As the player mixes down everything, we'll hear both files at the same time now:
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
                PlaybackStatusEvent::Position { 
                    id, 
                    path, 
                    context: _, 
                    position 
                } => {
                    println!(
                        "Playback pos of source #{id} '{path}': {pos}",
                        pos = position.as_secs_f32()
                    );
                }
                PlaybackStatusEvent::Stopped {
                    id,
                    path,
                    context: _,
                    exhausted,
                } => {
                    if exhausted {
                        println!("Playback of #{id} '{path}' finished");
                    } else {
                        println!("Playback of #{id} '{path}' was stopped");
                    }
                }
            }
        }
    });

    // Playing files can be seeked or stopped:
    player.seek_source(long_file_id, std::time::Duration::from_secs(5))?;
    player.stop_source(small_file_id)?;

    // If you only want one file to play at the same time, simply stop all playing
    // sounds before starting a new one:
    player.stop_all_sources()?;
    player.play_file("PATH_TO/boom.wav", FilePlaybackOptions::default())?;

    Ok(())
}
```

#### Playback Sequencing

Play a sample file sequence in time with e.g. musical beats.

```rust no_run
use phonic::{
   Player, OutputDevice, DefaultOutputDevice, Error, FilePlaybackOptions,
   utils::speed_from_note, PreloadedFileSource 
};

fn main() -> Result<(), Error> {
    // create a player
    let mut player = Player::new(DefaultOutputDevice::open()?.sink(), None);

    // calculate at which rate the sample file should be emitted
    let beats_per_min = 120.0;
    let samples_per_sec = player.output_sample_rate();
    let samples_per_beat = samples_per_sec as f64 * 60.0 / beats_per_min as f64;

    // preload a sample file
    let sample = PreloadedFileSource::from_file(
        "path/to_some_file.wav",
        None, // we don't need to track playback events here
        FilePlaybackOptions::default(),
        samples_per_sec,
    )?;

    // schedule playback of the sample file every beat for 8 beats
    let playback_start = player.output_sample_frame_position() as f64;
    for beat_counter in 0..8 {
        // when is the next beat playback due?
        let play_time = playback_start + (beat_counter as f64 * samples_per_beat);
        // play a clone of the preloaded sample at the next beat's sample time.
        // cloning is very cheap as the sample buffer is shared...
        player.play_file_source(
            sample.clone(
                FilePlaybackOptions::default()
                .speed(speed_from_note(60)), // middle-c
                samples_per_sec)?,
            Some(play_time as u64),
        )?;
    }

    Ok(())
}
```

#### DSP Effects and Mixer Graphs

Create complex audio processing chains by routing sources through different mixers and effects.

```rust no_run
use phonic::{
    Player, OutputDevice, DefaultOutputDevice, Error, FilePlaybackOptions,
    effect::{
        chorus::{ChorusEffect, ChorusEffectMessage},
        reverb::{ReverbEffect, ReverbEffectMessage},
    }
};

fn main() -> Result<(), Error> {
    // Create a player
    let mut player = Player::new(DefaultOutputDevice::open()?.sink(), None);

    // Add a reverb effect to the main mixer. All sounds played without a
    // specific target mixer will be routed through this effect.
    let reverb_id = player.add_effect(ReverbEffect::new(), None)?;
    player.send_effect_message(reverb_id, ReverbEffectMessage::SetWet(0.5))?;

    // Create a new sub-mixer that is a child of the main mixer.
    let chorus_mixer_id = player.add_mixer(None)?;
    // Add a chorus effect to this new mixer.
    let chorus_id = player.add_effect(ChorusEffect::new(), Some(chorus_mixer_id))?;
    player.send_effect_message(chorus_id, ChorusEffectMessage::SetWet(0.8))?;

    // Play a file through the main mixer (which has reverb).
    player.play_file(
        "PATH_TO/some_file.wav",
        FilePlaybackOptions::default(),
    )?;

    // Play another file, but route it through our chorus mixer.
    player.play_file(
        "PATH_TO/another_file.wav",
        FilePlaybackOptions::default().target_mixer(chorus_mixer_id),
    )?;

    Ok(())
}
```

## License

phonic is distributed under the terms of both the MIT license and the Apache License (Version 2.0).

* Apache License, Version 2.0, ([LICENSE-APACHE](LICENSE-APACHE) or [http://www.apache.org/licenses/LICENSE-2.0]())
* MIT license ([LICENSE-MIT](LICENSE-MIT) or [http://opensource.org/licenses/MIT]())
