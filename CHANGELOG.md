# Changelog

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
