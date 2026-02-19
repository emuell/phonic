# Changelog

## v0.15.0 - 2025/02/19

- add concurrent sub-mixer processing using a new channel-synced audio worker thread pool impl
- add options to `Player` and a new `new_with_config` function to configure concurrent processing
- add optional granular sample playback to `Sampler` (enabled with the `with_granular` builder function)
- [breaking] removed optional envelope parameter from `Sampler` (use the `with_ahdsr` builder function) 
- add basic volume, panning, transpose and finetune parameters to `Sampler`
- add optional parameter modulation support to the `Generator` trait
- add modulation matrix impls for `Sampler` (for granular parameters only) and `FunDspGenerator` (flexible)
- add new `play-grains` and `play-multi-threaded` examples
- correctly handle `nan` values in parameter string formatters 
- update emscripten example to test and showcase new synth generator modulation system

## v0.14.0 - 2025/01/14

- [breaking] replaced dasp with fundsp
- [breaking] effect parameter changes can now only be applied via new `EffectHandle`s
- [breaking] removed `ClonableParameter`, added a new `dyn_clone` function to `Parameter` instead
- add new `Generator` trait, a source which gets triggered by note/parameter change events:
  - Generators can be transient like Sources or can be added like effects to sub-mixers
  - Generators may have automatable parameters, just like effects 
  - add simple `Sampler` Generator impl which plays back a single sample with optional AHDSR envelopes
  - add parameterized `FunDspGenerator` impl and a few fundsp synth examples
- optionally measure CPU load of sources & mixers via new playback options
- add new `GuardedSource` and an optional panic hook for the player in order to gracefull deal with crashes
- add `Display` impl for player to debug dump Player's mixer, source and effect structure
- add Effect impl for `Box<dyn Effect>` so `dyn Effect`s can be added just like effect impls in the player
- add new `ParameterPolarity` property to `ParameterType`
- immediately send playback position events when starting sources
- updated examples to showcase new `FunDspGenerator` and `Sampler`
- refactored, speed up the airwindows Reverb effect implementation and fixed parameter smoothing
- fixed deserialization of `FloatParameter` and `IntegerParameter` values which have units

## v0.13.0 - 2025/11/26

- [breaking] `play_file/synth` and `add_effect` functions now return playback handles:
  - handles of file and synth sources allow checking if the source is still playing, and allow changing playback properties of the playing source.
  - handles of added effects allow scheduling/changing effect parameters.
  - Related functions in the player interface got removed from the player and can only be used with the handles.
  - all handles are `Send` + `Sync`, so they can be moved and used from other threads.  
- fix memory (de)alloc in the audio-thread when removing effects:
- fix bogus test if resampling is needed in ResampledSource. 
- fix missing check for already scheduled stop playing commands

## v0.12.0 - 2025/11/10

- auto-bypass DSP effects that receive no input signals to save CPU cycles
- allow moving DSP effects within the mixer
- [breaking] use std::mpsc::sync_channel instead of crossbeam_channel for playback event tracking 
- disable unused dependency features to reduce bloat
- fix incorrect sample rate used in playback position events

## v0.11.2 - 2025/10/19

- fix possible never-ending loop when looking up loop chunk data in broken RIFF files
- allow accessing shared file buffers of PreloadedFileSources and construction
 
## v0.11.1 - 2025/10/14

- fixed docs.rs builds
- fixed handling of "past" scheduled source stop events in mixer

## v0.11.0 - 2025/10/10

- finalized effect traits and implemented basic set of stock effects: gain, filter, EQ, reverb, chorus, compressor, limiter and distortion
- updated emscripten example to test and showcase effects

## v0.10.0 - 2025/09/27

- only enable audio codecs and format features in symphonia, skip video containers, to reduce bloat
- enable logging and optional wav output arguments for all examples 
- replaced emscripten sokol audio output backend with a new custom one
- allow changing and scheduling of volume/panning in playing sources
- fixed some edge cases in resampler input constraints

## v0.9.1 - 2025/09/12

- fixed compile errors on docs.rs

## v0.9.0 - 2025/09/12

- add nested sub-mixer and DSP effect support
- add wav writer output device
- add set of basic built in effects (chorus, filter, compressor/limiter, reverb)
- reorganized public exports
- added new and updated existing examples
- updated crate docs
- changed license to GNU AFFERO

## v0.8.0 - 2025/09/08

- add real-time, glided file playback speed changes 
- add new SmoothedValue trait and impls to smoothly change parameter values such as volume
- read and apply loop points from WAV and FLAC files by default 
- fixed broken rubato resampler impl (applied with the high quality resampling playback option)

## v0.7.1 - 2025/07/16

- fixed missing support for other cpal output formats than f32
- update cpal to v0.16

## v0.7.0 - 2025/06/16

- add file playback from raw encoded file buffers
- add global `volume` factor setters to player
- fixed volume `fade_in/out` duration calculation 

## v0.6.1 - 2025/06/02

- fixed `add_buffers_with_simd` impls

## v0.6.0 - 2025/05/31

- add new `PannedSource` and use it to apply new `panning` file/synth playback properties
- speed up basic buffer operations with simd via `pulp`

## v0.5.1 - 2025/05/28

- fixed processing of exhausted sources in `MixedSource` and cleaned up its impl
- use a custom version of `sokol` which uses the latest version of `cc` to fix incompatibilities with other crates using cc

## v0.5.0 - 2025/05/24

- new `suspended` in `OutputSink` to check if web audio output currently is suspended by the browser.
- fixed wrong calculated `playback_pos` in Sokol audio output

## v0.4.0 - 2025/05/22

- new `waveform` utilities

## v0.3.0 - 2025/05/18

_initial public release._
