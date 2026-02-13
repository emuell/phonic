//! Granular playback impl for Sampler.

use std::sync::{Arc, LazyLock};

use rand::{rngs::SmallRng, Rng, SeedableRng};

use assume::assume;
use strum::EnumCount;

use crate::{utils::buffer::InterleavedBufferMut, Error};

// -------------------------------------------------------------------------------------------------

/// Playback direction for grains.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::Display, strum::VariantNames,
)]
#[repr(u8)]
pub enum GrainPlaybackDirection {
    /// Play grains forward through the file.
    Forward,
    /// Play grains backward through the file.
    Backward,
    /// Play grains in a random direction.
    Random,
}

// -------------------------------------------------------------------------------------------------

/// Playhead mode for granular synthesis grain position tracking.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::Display, strum::VariantNames,
)]
#[repr(u8)]
pub enum GrainPlayheadMode {
    /// Grains spawn at a fixed manual position (default 0.5 = middle of file).
    /// Spray parameter adds randomness around this position.
    Manual,
    /// Grains spawn at a position that advances through the file over time.
    /// Creates a "moving window" effect. Spray still adds randomness.
    PlayThrough,
}

/// Grain overlap mode for controlling how grains are scheduled.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, strum::EnumString, strum::Display, strum::VariantNames,
)]
#[repr(u8)]
pub enum GrainOverlapMode {
    /// Multiple grains can overlap freely (current behavior).
    /// Grains trigger at density-based intervals.
    /// Up to POOL_SIZE concurrent grains.
    Cloud,
    /// Queue-based playback with adaptive crossfading.
    /// New grain triggers when current grain reaches its crossfade point.
    /// Maximum 2 grains active during crossfade.
    /// grain_density parameter is ignored.
    Sequential,
}

/// Grain window mode selection (optimized for granular synthesis)
/// Ordered by smoothness: smooth → balanced → sharp → rhythmic
#[derive(
    Clone,
    Copy,
    Debug,
    PartialEq,
    Eq,
    strum::EnumString,
    strum::Display,
    strum::VariantNames,
    strum::EnumCount,
)]
#[repr(u8)]
pub enum GrainWindowMode {
    Hann = 0,
    Blackman = 1,
    Triangle = 2,
    Tukey = 3,
    Trapezoid = 4,
    Exponential = 5,
    RampUp = 6,
    RampDown = 7,
}

impl GrainWindowMode {
    /// Get the crossfade trigger point (0.0-1.0) for sequential playback.
    /// Returns the grain progress percentage at which the next grain should trigger.
    ///
    /// Smooth windows need early crossfade for gap-free playback, while
    /// sharp/sustaining windows can wait longer for maximum grain separation.
    pub fn sequential_crossfade_point(&self) -> f32 {
        match self {
            // Smooth windows need early crossfade for gap-free playback
            GrainWindowMode::Hann
            | GrainWindowMode::Blackman
            | GrainWindowMode::Triangle
            | GrainWindowMode::Tukey => 0.5,

            // Trapezoid has sustain, can wait until near the end
            GrainWindowMode::Trapezoid => 0.9,

            // Pointed/asymmetric windows
            GrainWindowMode::Exponential | GrainWindowMode::RampUp | GrainWindowMode::RampDown => {
                0.8
            }
        }
    }
}

/// Precomputed grain windows (optimized for granular synthesis)
/// `N` must be a pow2 value.
pub(crate) struct GrainWindow<const N: usize> {
    luts: [[f32; N]; GrainWindowMode::COUNT],
}

impl<const N: usize> GrainWindow<N> {
    /// Calculate bit mask from N
    const _VERIFY_N: () = assert!(
        N.is_power_of_two(),
        "Grain window size must be a pow2 value"
    );
    const MASK: usize = N - 1;

    /// Precompute all window LUTs
    pub fn new() -> Self {
        let mut luts = [[0.0; N]; GrainWindowMode::COUNT];

        #[allow(clippy::needless_range_loop)]
        for i in 0..N {
            let phase = i as f32 / N as f32; // [0.0, 1.0)

            // Hann: cosine-squared window (perfect overlap-add)
            // Standard for granular synthesis
            // Also known as Hanning window
            luts[GrainWindowMode::Hann as usize][i] =
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * phase).cos());

            // Blackman: classic DSP window with steep spectral rolloff
            // a0=0.42, a1=0.5, a2=0.08 (standard coefficients)
            // Smooth spectral transitions, wider main lobe
            let pi_phase = std::f32::consts::PI * phase;
            luts[GrainWindowMode::Blackman as usize][i] =
                0.42 - 0.5 * (2.0 * pi_phase).cos() + 0.08 * (4.0 * pi_phase).cos();

            // Triangle: linear rise to peak at 0.5, linear fall
            luts[GrainWindowMode::Triangle as usize][i] = if phase < 0.5 {
                2.0 * phase
            } else {
                2.0 * (1.0 - phase)
            };

            // Tukey: tapered cosine (truncation α = 0.5)
            // Variable sustain for longer grains
            // Morphs from rectangular to fully-cosine-tapered
            let alpha = 0.5;
            let width = alpha / 2.0;
            luts[GrainWindowMode::Tukey as usize][i] = if phase < width {
                let u = phase / width;
                0.5 * (1.0 - (std::f32::consts::PI * u).cos())
            } else if phase > 1.0 - width {
                let u = (1.0 - phase) / width;
                0.5 * (1.0 - (std::f32::consts::PI * u).cos())
            } else {
                1.0
            };

            // Trapezoid: linear ramps with flat sustain (~80% sustain)
            // Percussive with clear transients, punchy
            let ramp_width = 0.1;
            luts[GrainWindowMode::Trapezoid as usize][i] = if phase < ramp_width {
                phase / ramp_width
            } else if phase > 1.0 - ramp_width {
                (1.0 - phase) / ramp_width
            } else {
                1.0
            };

            // Exponential: non-linear decay from center (Poisson window)
            // Creates "push" character with emphasis on center
            // Rhythmic emphasis, pointed grains
            let decay_rate = 6.0;
            let center_dist = (phase - 0.5).abs();
            luts[GrainWindowMode::Exponential as usize][i] = (-decay_rate * center_dist).exp();

            // Ramp Up: 90% linear rise, 10% quick cosine fade
            // Strong upward movement, rhythmic effects
            luts[GrainWindowMode::RampUp as usize][i] = if phase < 0.9 {
                // Linear rise over 90%
                phase / 0.9
            } else {
                // Quick cosine fade over last 10%
                let u = (phase - 0.9) / 0.1;
                0.5 * (1.0 + (std::f32::consts::PI * u).cos())
            };

            // Ramp Down: 10% quick cosine rise, 90% linear fall
            // Strong downward movement, rhythmic effects
            luts[GrainWindowMode::RampDown as usize][i] = if phase < 0.1 {
                // Quick cosine rise over first 10%
                let u = phase / 0.1;
                0.5 * (1.0 - (std::f32::consts::PI * u).cos())
            } else {
                // Linear fall over remaining 90%
                1.0 - ((phase - 0.1) / 0.9)
            };
        }

        Self { luts }
    }

    /// Evaluate a window at normalized phase [0.0, 1.0]
    /// Uses linear interpolation for smooth lookup between LUT samples
    #[inline]
    pub fn sample(&self, mode: GrainWindowMode, phase: f64) -> f32 {
        debug_assert!((0.0..=1.0).contains(&phase));

        let index_float = phase * (N - 1) as f64;
        let index = (index_float as usize) & Self::MASK;
        let fraction = index_float.fract() as f32;
        let next_index = (index + 1) & Self::MASK;

        let lut = &self.luts[mode as usize];
        if index < N - 1 {
            lut[index] * (1.0 - fraction) + lut[next_index] * fraction
        } else {
            lut[N - 1]
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Modulation buffers for block-based parameter modulation.
/// Contains pre-computed modulation values for a block of samples.
pub(crate) struct GranularParameterModulation<'a> {
    pub size: &'a [f32],
    pub density: &'a [f32],
    pub variation: &'a [f32],
    pub spray: &'a [f32],
    pub pan_spread: &'a [f32],
    pub position: &'a [f32],
    pub speed: &'a [f32],
}

// -------------------------------------------------------------------------------------------------

/// Parameters controlling granular playback behavior.
#[derive(Clone, Debug)]
pub struct GranularParameters {
    /// Grain overlap mode (Cloud or Sequential).
    pub overlap_mode: GrainOverlapMode,
    /// Grain window mode.
    pub window: GrainWindowMode,
    /// Size of each grain in milliseconds (1.0 - 1000.0).
    pub size: f32,
    /// Density of grain spawning in Hz (1.0 - 100.0).
    /// Represents the number of new grains triggered per second.
    pub density: f32,
    /// Grain variation (0.0 = no variation, 1.0 = full variation of size and volume)
    /// At 1.0, grain size varies 25%-200% and volume varies 0.0-1.0.
    pub variation: f32,
    /// Random variation in grain start position (0.0 - 1.0).
    /// Each grain's start position is varied by ±2.0 seconds at maximum spray.
    pub spray: f32,
    /// Random stereo spread per grain (0.0 - 1.0).
    /// Each grain's panning is offset by ±(pan_spread × 0.5) from the voice's base pan.
    pub pan_spread: f32,
    /// Direction for grain playback (forward, backward, or random).
    pub playback_direction: GrainPlaybackDirection,
    /// Playhead mode for grain position tracking (Manual or PlayThrough).
    pub playhead_mode: GrainPlayheadMode,
    /// Manual position in the file (0.0 - 1.0) when playhead_mode is Manual.
    pub manual_position: f32,
    /// Playback speed multiplier for PlayThrough mode (typically 0.1 - 4.0).
    pub playhead_speed: f32,
}

impl Default for GranularParameters {
    fn default() -> Self {
        Self {
            overlap_mode: GrainOverlapMode::Cloud,
            window: GrainWindowMode::Triangle,
            size: 100.0,
            density: 10.0,
            spray: 0.0,
            variation: 0.0,
            pan_spread: 0.0,
            playback_direction: GrainPlaybackDirection::Forward,
            playhead_mode: GrainPlayheadMode::Manual,
            manual_position: 0.5,
            playhead_speed: 1.0,
        }
    }
}

impl GranularParameters {
    pub fn new() -> Self {
        Self::default()
    }

    /// Validate all parameters.
    pub fn validate(&self) -> Result<(), Error> {
        if self.size < 1.0 || self.size > 1000.0 {
            return Err(Error::ParameterError(
                "Grain size must be between 1 and 1000 ms".to_string(),
            ));
        }

        if self.density < 1.0 || self.density > 100.0 {
            return Err(Error::ParameterError(
                "Grain density must be between 1.0 and 100.0 Hz".to_string(),
            ));
        }

        if self.spray < 0.0 || self.spray > 1.0 {
            return Err(Error::ParameterError(
                "Grain spray must be between 0.0 and 1.0".to_string(),
            ));
        }

        if self.variation < 0.0 || self.variation > 1.0 {
            return Err(Error::ParameterError(
                "Grain variation must be between 0.0 and 1.0".to_string(),
            ));
        }

        if self.pan_spread < 0.0 || self.pan_spread > 1.0 {
            return Err(Error::ParameterError(
                "Grain pan spread must be between 0.0 and 1.0".to_string(),
            ));
        }

        if self.manual_position < 0.0 || self.manual_position > 1.0 {
            return Err(Error::ParameterError(
                "Manual position must be between 0.0 and 1.0".to_string(),
            ));
        }

        if self.playhead_speed < 0.001 || self.playhead_speed > 4.0 {
            return Err(Error::ParameterError(
                "Playhead speed must be between 0.001 and 4.0".to_string(),
            ));
        }

        Ok(())
    }
}

// -------------------------------------------------------------------------------------------------

/// Manages granular synthesis playback by spawning and mixing up to `POOL_SIZE` concurrent grains.
///
/// Each grain is a short windowed segment of audio with its own position, pitch, and envelope.
/// New grains are triggered at a rate determined by [GranularParameters::grain_density_hz],
/// with their start positions controlled by the playhead mode:
/// - [GrainPlayheadMode::Manual]: All grains spawn around a fixed position
/// - [GrainPlayheadMode::PlayThrough]: Grains spawn from an advancing playhead
///
/// The pool reuses inactive [Grain] instances to avoid allocations during real-time processing.
/// Active grains are processed in parallel each sample, reading from the source buffer and
/// mixing their output with sine-windowed envelopes applied.
pub(crate) struct GrainPool<const POOL_SIZE: usize> {
    /// Current overlap mode (Cloud or Sequential).
    overlap_mode: GrainOverlapMode,
    /// Pool of reusable grain instances.
    grain_pool: [Grain; POOL_SIZE],
    /// Indices of currently active grains.
    active_grain_indices: Vec<usize>,
    /// Index of primary grain in Sequential mode (for tracking crossfade point).
    primary_grain_index: Option<usize>,
    /// Grain source buffer (a resampled, decoded mono sample buffer)
    sample_buffer: Arc<Box<[f32]>>,
    /// Loop range for playback (normalized 0.0..1.0).
    sample_loop_range: Option<(f32, f32)>,
    /// Whether new grains should be triggered (set to false when stopping).
    trigger_new_grains: bool,
    /// Current phase of the grain trigger oscillator (0.0..1.0).
    /// Increments based on grain_density_hz to determine when to spawn new grains.
    trigger_phase: f32,
    /// Playback speed/pitch multiplier for all grains.
    speed: f64,
    /// Overall volume multiplier for all grains (0.0..1.0+).
    volume: f32,
    /// Base stereo panning position for grains (-1.0..1.0).
    panning: f32,
    /// Current playhead position for PlayThrough mode (0.0..1.0).
    /// Advances through the file over time, determining where new grains spawn.
    playhead: f32,
    /// Sample rate of the audio output.
    sample_rate: u32,
    /// Random number generator for spray and pan spread variations.
    rng: SmallRng,
}

/// Static, shared lookup table for the envelope window modes
static GRAIN_WINDOW_LUT: LazyLock<GrainWindow<2048>> = LazyLock::new(GrainWindow::new);

impl<const POOL_SIZE: usize> GrainPool<POOL_SIZE> {
    /// Minimum envelope amplitude threshold below which grains are skipped.
    const ENVELOPE_THRESHOLD: f32 = 0.001; // ~ -60dB

    /// Create a new grain pool with the given sample rate, source sample buffer and optional loop points.
    pub fn new(
        sample_rate: u32,
        sample_buffer: Arc<Box<[f32]>>,
        sample_loop_range: Option<(f32, f32)>,
    ) -> Self {
        debug_assert!(
            !sample_buffer.is_empty(),
            "Need a valid, non empty sample buffer"
        );
        debug_assert!(
            sample_loop_range
                .is_none_or(|l| (0.0..=1.0).contains(&l.0) && (0.0..=1.0).contains(&l.1)),
            "Invalid loop points (should be relative positions), but are: {:?}",
            sample_loop_range
        );
        let overlap_mode = GrainOverlapMode::Cloud;
        let grain_pool = [Grain::new(); POOL_SIZE];
        let active_grain_indices = Vec::with_capacity(POOL_SIZE);
        let primary_grain_index = None;
        let trigger_phase = 0.0;
        let trigger_new_grains = true;
        let speed = 1.0;
        let volume = 1.0;
        let panning = 0.0;
        let playhead = 0.0;
        let rng = SmallRng::from_os_rng();

        Self {
            overlap_mode,
            grain_pool,
            active_grain_indices,
            primary_grain_index,
            sample_buffer,
            sample_loop_range,
            trigger_new_grains,
            trigger_phase,
            speed,
            volume,
            panning,
            playhead,
            sample_rate,
            rng,
        }
    }

    pub fn is_exhausted(&self) -> bool {
        !self.trigger_new_grains && self.active_grain_indices.is_empty()
    }

    pub fn playback_position(&self, parameters: &GranularParameters, position_mod: f32) -> f32 {
        // Determine base position based on playhead mode
        let mut base_position = match parameters.playhead_mode {
            GrainPlayheadMode::Manual => parameters.manual_position,
            GrainPlayheadMode::PlayThrough => self.playhead,
        };

        // Apply modulation
        base_position += position_mod;

        // Fold manual position into loop range
        if parameters.playhead_mode == GrainPlayheadMode::Manual {
            if let Some((loop_start, loop_end)) = self.sample_loop_range {
                let loop_len = loop_end - loop_start;
                if loop_len > 0.0 {
                    base_position = loop_start + (base_position - loop_start).rem_euclid(loop_len);
                }
            }
        }
        // Return modulated position
        base_position.rem_euclid(1.0)
    }

    pub fn start(&mut self, speed: f64, volume: f32, panning: f32) {
        self.trigger_new_grains = true;
        self.trigger_phase = 1.0;

        self.speed = speed;
        self.volume = volume;
        self.panning = panning;
        self.playhead = 0.0;
    }

    pub fn stop(&mut self) {
        self.trigger_new_grains = false;
    }

    pub fn reset(&mut self) {
        self.active_grain_indices.clear();
        for grain in &mut self.grain_pool {
            grain.deactivate();
        }
        self.trigger_new_grains = true;
        self.primary_grain_index = None;
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.speed = speed;
    }

    pub fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
    }

    pub fn set_panning(&mut self, panning: f32) {
        self.panning = panning;
    }

    /// Try to trigger a new grain if the trigger phase indicates it's time.
    /// Returns true if a grain was triggered.
    #[inline]
    #[allow(clippy::too_many_arguments)]
    fn try_trigger_grain(
        &mut self,
        parameters: &GranularParameters,
        size_mod: f32,
        density_mod: f32,
        variation_mod: f32,
        spray_mod: f32,
        pan_spread_mod: f32,
        position_mod: f32,
    ) -> bool {
        // Detect mode changes
        if self.overlap_mode != parameters.overlap_mode {
            self.overlap_mode = parameters.overlap_mode;
            self.primary_grain_index = None;
        }

        // Sequential mode: check if primary grain has reached crossfade point
        if self.overlap_mode == GrainOverlapMode::Sequential {
            if let Some(primary_index) = self.primary_grain_index {
                let primary_grain = &self.grain_pool[primary_index];
                if primary_grain.is_active() {
                    // Calculate grain progress (window_phase ranges 0.0-1.0)
                    let grain_progress = primary_grain.window_phase();
                    let crossfade_point = parameters.window.sequential_crossfade_point();

                    // Block new grain until primary reaches crossfade point
                    if grain_progress < crossfade_point as f64 {
                        return false;
                    }
                }
            }
        }

        if !self.trigger_new_grains || !self.update_trigger_phase(parameters, density_mod) {
            return false;
        }

        // Calculate playback position
        let modulated_position = self.playback_position(parameters, position_mod);

        // Apply spray to randomize grain start position
        let spray_variation = if !self.sample_buffer.is_empty() {
            let file_duration = self.sample_buffer.len() as f64 / self.sample_rate as f64;
            // Apply modulation to spray (additive, clamped)
            let modulated_spray = (parameters.spray + spray_mod).clamp(0.0, 1.0);
            // Spray range: +/- 2.0 seconds at 1.0
            let spray_seconds = modulated_spray as f64 * 4.0 * (self.rng.random::<f64>() - 0.5);
            spray_seconds / file_duration
        } else {
            0.0
        };
        let grain_position = (modulated_position as f64 + spray_variation).rem_euclid(1.0);

        // Start a new grain
        let activated_index = self.activate_new_grain(
            parameters,
            size_mod,
            variation_mod,
            pan_spread_mod,
            grain_position,
        );

        // In Sequential mode, track the primary grain for crossfade timing
        if self.overlap_mode == GrainOverlapMode::Sequential {
            if let Some(index) = activated_index {
                self.primary_grain_index = Some(index);
            }
        }

        activated_index.is_some()
    }

    /// Advance the playhead position for PlayThrough mode.
    #[inline]
    fn advance_playhead(&mut self, buffer_frame_count: usize, playhead_speed: f32, speed_mod: f32) {
        // Apply modulation to playhead speed (multiplicative)
        let speed_mult = 1.0 + speed_mod;
        let modulated_speed = playhead_speed * speed_mult;

        // Advance position by one frame worth of time at the current playback speed
        let position_increment = modulated_speed / buffer_frame_count as f32;
        self.playhead += position_increment;

        // Wrap around at file boundaries or loop points
        if let Some((loop_start, loop_end)) = self.sample_loop_range {
            if self.playhead >= loop_end {
                let loop_len = loop_end - loop_start;
                if loop_len > 0.0 {
                    self.playhead = loop_start + (self.playhead - loop_end) % loop_len;
                } else {
                    self.playhead = loop_start;
                }
            } else if self.playhead < loop_start {
                let loop_len = loop_end - loop_start;
                if loop_len > 0.0 {
                    self.playhead = loop_end - (loop_start - self.playhead).rem_euclid(loop_len);
                } else {
                    self.playhead = loop_start;
                }
            }
        } else if self.playhead >= 1.0 {
            self.playhead -= 1.0;
        } else if self.playhead < 0.0 {
            self.playhead += 1.0;
        }
    }

    pub fn process(
        &mut self,
        mut output: &mut [f32],
        channel_count: usize,
        parameters: &GranularParameters,
        modulation: &GranularParameterModulation,
    ) -> usize {
        let grain_window = &*GRAIN_WINDOW_LUT;

        let sample_frame_count = self.sample_buffer.len();
        let move_playhead =
            parameters.playhead_mode == GrainPlayheadMode::PlayThrough && sample_frame_count > 0;

        // Eliminate channel count match branch from hot path
        match channel_count {
            1 => {
                // Mono processing
                for (frame_index, frame) in output.as_frames_mut::<1>().iter_mut().enumerate() {
                    // Trigger new grains with modulated parameters
                    self.try_trigger_grain(
                        parameters,
                        modulation.size[frame_index],
                        modulation.density[frame_index],
                        modulation.variation[frame_index],
                        modulation.spray[frame_index],
                        modulation.pan_spread[frame_index],
                        modulation.position[frame_index],
                    );
                    // Move Playhead
                    if move_playhead {
                        self.advance_playhead(
                            sample_frame_count,
                            parameters.playhead_speed,
                            modulation.speed[frame_index],
                        );
                    }
                    // Process all active grains and mix to mono output
                    for &grain_index in &self.active_grain_indices {
                        let grain = &mut self.grain_pool[grain_index];
                        if !grain.is_active() {
                            continue;
                        }
                        let grain_output = grain.process(grain_window);
                        if grain_output.envelope > Self::ENVELOPE_THRESHOLD {
                            let sample = self.sample_at_position(grain_output.position);
                            frame[0] += sample * grain_output.envelope;
                        }
                    }
                }
            }
            2 => {
                // Stereo processing
                for (frame_index, frame) in output.as_frames_mut::<2>().iter_mut().enumerate() {
                    // Trigger new grains with modulated parameters
                    self.try_trigger_grain(
                        parameters,
                        modulation.size[frame_index],
                        modulation.density[frame_index],
                        modulation.variation[frame_index],
                        modulation.spray[frame_index],
                        modulation.pan_spread[frame_index],
                        modulation.position[frame_index],
                    );
                    // Move Playhead
                    if move_playhead {
                        self.advance_playhead(
                            sample_frame_count,
                            parameters.playhead_speed,
                            modulation.speed[frame_index],
                        );
                    }
                    // Process all active grains and mix to stereo output
                    for &grain_index in &self.active_grain_indices {
                        let grain = &mut self.grain_pool[grain_index];
                        if grain.is_active() {
                            let grain_output = grain.process(grain_window);
                            if grain_output.envelope > Self::ENVELOPE_THRESHOLD {
                                let sample = self.sample_at_position(grain_output.position);
                                let windowed_sample = sample * grain_output.envelope;

                                let left_gain = (1.0 - grain_output.panning) * 0.5;
                                let right_gain = (1.0 + grain_output.panning) * 0.5;
                                frame[0] += windowed_sample * left_gain;
                                frame[1] += windowed_sample * right_gain;
                            }
                        }
                    }
                }
            }
            _ => {
                // Multi-channel processing (only modify first two channels)
                for (frame_index, frame) in output.frames_mut(channel_count).enumerate() {
                    // Trigger new grains
                    self.try_trigger_grain(
                        parameters,
                        modulation.size[frame_index],
                        modulation.density[frame_index],
                        modulation.variation[frame_index],
                        modulation.spray[frame_index],
                        modulation.pan_spread[frame_index],
                        modulation.position[frame_index],
                    );
                    // Move Playhead
                    if move_playhead {
                        self.advance_playhead(
                            sample_frame_count,
                            parameters.playhead_speed,
                            modulation.speed[frame_index],
                        );
                    }
                    // Process all active grains on a temp stereo output pair
                    let mut stereo_out = [0.0; 2];
                    for &grain_index in &self.active_grain_indices {
                        let grain = &mut self.grain_pool[grain_index];
                        if !grain.is_active() {
                            continue;
                        }
                        let grain_output = grain.process(grain_window);
                        if grain_output.envelope > Self::ENVELOPE_THRESHOLD {
                            let sample = self.sample_at_position(grain_output.position);
                            let windowed_sample = sample * grain_output.envelope;

                            let left_gain = (1.0 - grain_output.panning) * 0.5;
                            let right_gain = (1.0 + grain_output.panning) * 0.5;
                            stereo_out[0] += windowed_sample * left_gain;
                            stereo_out[1] += windowed_sample * right_gain;
                        }
                    }
                    // Copy stereo output pair
                    for (channel, sample) in frame.enumerate() {
                        if channel < 2 {
                            *sample += stereo_out[channel];
                        }
                    }
                }
            }
        }

        // Cleanup grains from the list which finished playback
        self.active_grain_indices
            .retain(|&index| self.grain_pool[index].is_active());

        output.len()
    }

    /// Get the current grain trigger phase for density-based grain spawning.
    /// Returns true if a grain should be triggered in this sample.
    fn update_trigger_phase(
        &mut self,
        granular_params: &GranularParameters,
        density_mod: f32,
    ) -> bool {
        // Sequential mode triggers new grains as soon as the old one finished
        if self.overlap_mode == GrainOverlapMode::Sequential {
            return true;
        }
        // Density: bipolar modulation, multiplies current density
        let density_mult = 1.0 + density_mod;
        let density = (granular_params.density * density_mult).clamp(1.0, 100.0);

        let trigger_increment = density / self.sample_rate as f32;
        self.trigger_phase += trigger_increment;

        if self.trigger_phase >= 1.0 {
            self.trigger_phase -= 1.0;
            return true;
        }
        false
    }

    /// Activate a new grain at the given position with the voice's current pitch.
    /// Returns Some(index) if a grain was successfully activated, None if no free grains available.
    fn activate_new_grain(
        &mut self,
        parameters: &GranularParameters,
        size_mod: f32,
        variation_mod: f32,
        pan_spread_mod: f32,
        position: f64,
    ) -> Option<usize> {
        if let Some(index) = self.grain_pool.iter().position(|g| !g.is_active()) {
            let grain = &mut self.grain_pool[index];
            let window_mode = parameters.window;
            let speed = self.speed;

            // Apply modulation to variation (additive, clamped)
            let variation = (parameters.variation + variation_mod).clamp(0.0, 1.0);

            // Volume variation: 1.0 -> 0..1, 0.0 -> 1.0
            let volume_scale = 1.0 - (variation * self.rng.random::<f32>());
            let volume = self.volume * volume_scale;

            // Grain size variation: 1.0 -> 25%..400%
            let min_scale = 1.0 - (0.75 * variation);
            let max_scale = 1.0 + (2.0 * variation);
            let size_scale = min_scale + (max_scale - min_scale) * self.rng.random::<f32>();

            // Size: bipolar modulation, multiplies current size
            // Convert mod value to multiplier: -1 → 0.5×, 0 → 1.0×, +1 → 2.0×
            let size_mult = 1.0 + size_mod;
            let grain_size_ms = (parameters.size * size_mult).clamp(1.0, 1000.0);

            let grain_size =
                ((grain_size_ms * size_scale * self.sample_rate as f32 / 1000.0) as usize).max(2);

            // Apply modulation to pan_spread (additive, clamped)
            let modulated_pan_spread = (parameters.pan_spread + pan_spread_mod).clamp(0.0, 1.0);
            let panning_spread = modulated_pan_spread * (self.rng.random::<f32>() * 2.0 - 1.0);
            let panning = (self.panning + panning_spread).clamp(-1.0, 1.0);

            let file_length_frames = self.sample_buffer.len();
            let reverse = match parameters.playback_direction {
                GrainPlaybackDirection::Forward => false,
                GrainPlaybackDirection::Backward => true,
                GrainPlaybackDirection::Random => self.rng.random::<bool>(),
            };
            grain.activate(
                window_mode,
                position,
                speed,
                volume,
                panning,
                grain_size,
                file_length_frames,
                reverse,
            );
            if let Some(position) = self.active_grain_indices.iter().position(|&v| v == index) {
                // don't recycle a grain when it got stopped in the current process cycle
                self.active_grain_indices.remove(position);
            }
            self.active_grain_indices.push(index);
            Some(index)
        } else {
            None
        }
    }

    /// Sample from the file at a normalized position (0.0-1.0) using cubic interpolation.
    #[inline]
    fn sample_at_position(&self, normalized_pos: f32) -> f32 {
        let len = self.sample_buffer.len();

        assume!(unsafe: len > 0, "Buffer len is asserted in constructor");
        let max_index = len - 1;
        let float_index = normalized_pos * max_index as f32;

        let index = (float_index as usize).min(max_index);
        let fraction = float_index - (index as f32);

        // Calculate indices for 4-point cubic interpolation
        let i1 = index;
        let i2 = if i1 < max_index { i1 + 1 } else { 0 };
        let i0 = if i1 > 0 { i1 - 1 } else { max_index };
        let i3 = if i2 < max_index { i2 + 1 } else { 0 };

        assume!(unsafe: i0 < len);
        let y0 = self.sample_buffer[i0];
        assume!(unsafe: i1 < len);
        let y1 = self.sample_buffer[i1];
        assume!(unsafe: i2 < len);
        let y2 = self.sample_buffer[i2];
        assume!(unsafe: i3 < len);
        let y3 = self.sample_buffer[i3];

        // Cubic interpolation (Catmull-Rom)
        let a = -0.5 * y0 + 1.5 * y1 - 1.5 * y2 + 0.5 * y3;
        let b = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
        let c = -0.5 * y0 + 0.5 * y2;
        let d = y1;

        a * fraction * fraction * fraction + b * fraction * fraction + c * fraction + d
    }
}

// -------------------------------------------------------------------------------------------------

/// Single sample processing result of a [Grain].
///
/// Contains the envelope amplitude, stereo panning, and normalized file position
/// for a grain at a specific moment in time. This is the output of [Grain::process]
/// and is used by [GrainPool] to read and mix samples from the audio buffer.
#[derive(Debug, Copy, Clone)]
struct GrainOutput {
    /// Amplitude envelope value (0.0..1.0), combining grain volume and sine window.
    envelope: f32,
    /// Stereo panning position (-1.0 = full left, 0.0 = center, 1.0 = full right).
    panning: f32,
    /// Normalized position in the audio file (0.0 = start, 1.0 = end).
    position: f32,
}

// -------------------------------------------------------------------------------------------------

/// Represents a single grain of audio.
///
/// A grain is a short burst of audio with a smooth sine-based amplitude envelope.
/// Grains are spawned at regular intervals (density) and processed in parallel,
/// allowing for polyphonic granular synthesis effects.
#[derive(Debug, Clone, Copy)]
struct Grain {
    /// Is this grain currently active?
    active: bool,
    /// Grain's overall volume. May be randomized when there's a volume spread.
    volume: f32,
    /// Grain's panning position. May be randomized when there's a pan spread.
    panning: f32,
    /// Current playback position in the file (0.0 = start, 1.0 = end).
    /// This is independent of the voice's playback position.
    position: f64,
    /// Increment to apply to position each sample.
    /// Determined by grain pitch and playback direction.
    increment: f64,
    /// Number of samples remaining in this grain.
    /// When this reaches 0, the grain deactivates.
    samples_remaining: usize,
    /// Current position of the window envelope (0.0 to 1.0).
    window_phase: f64,
    /// Amount to increment envelope_phase each sample.
    window_increment: f64,
    /// Grain window type that should be applied.
    window_mode: GrainWindowMode,
}

impl Default for Grain {
    fn default() -> Self {
        Self::new()
    }
}

impl Grain {
    /// Create a new inactive grain.
    pub const fn new() -> Self {
        Self {
            active: false,
            position: 0.0,
            volume: 1.0,
            panning: 0.0,
            increment: 0.0,
            samples_remaining: 0,
            window_phase: 0.0,
            window_increment: 0.0,
            window_mode: GrainWindowMode::Triangle,
        }
    }

    /// Check if this grain is currently active.
    #[inline]
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Get current window phase (0.0-1.0) indicating grain progress.
    /// Used for sequential mode crossfade triggering.
    #[inline]
    pub fn window_phase(&self) -> f64 {
        self.window_phase
    }

    /// Activate this grain with the given parameters.
    #[allow(clippy::too_many_arguments)]
    pub fn activate(
        &mut self,
        window_mode: GrainWindowMode,
        position: f64,
        speed: f64,
        volume: f32,
        panning: f32,
        grain_size_samples: usize,
        file_length_frames: usize,
        reverse: bool,
    ) {
        self.active = true;
        self.window_mode = window_mode;
        self.position = position.clamp(0.0, 1.0);
        self.volume = volume.clamp(0.0, 100.0);
        self.panning = panning.clamp(-1.0, 1.0);
        self.samples_remaining = grain_size_samples;

        // Calculate the increment per sample
        // For a normalized position (0.0 to 1.0) spanning file_length_frames:
        // - At speed = 1.0: traverse the entire file (1.0) in file_length_frames samples
        // - increment = 1.0 / file_length_frames per sample
        // - With speed: increment = speed / file_length_frames
        let base_increment = if file_length_frames > 0 {
            speed / file_length_frames as f64
        } else {
            0.0
        };

        self.increment = base_increment * if reverse { -1.0 } else { 1.0 };

        // Initialize sine window envelope
        // The envelope will go from 0 to π during the grain's lifetime
        self.window_phase = 0.0;
        if grain_size_samples > 0 {
            // Increment to traverse the whole envelope in grain_size_samples steps
            self.window_increment = 1.0 / grain_size_samples as f64;
        } else {
            self.window_increment = 0.0;
        }
    }

    /// Deactivate this grain immediately.
    #[allow(dead_code)]
    pub fn deactivate(&mut self) {
        self.active = false;
        self.samples_remaining = 0;
    }

    /// Process this grain for one sample.
    ///
    /// Returns (envelope_value, position) for this sample.
    /// The caller should use this to read from the audio file at `position`
    /// and multiply the sample by `envelope_value`.
    pub fn process(&mut self, grain_window: &GrainWindow<2048>) -> GrainOutput {
        #[cfg(not(test))]
        debug_assert!(self.active, "Should only process active grains");

        let envelope_value = grain_window.sample(self.window_mode, self.window_phase);

        // Store current position for the caller to read the sample
        let position = self.position as f32;

        // Advance to next sample
        self.position += self.increment;
        self.window_phase += self.window_increment;
        self.samples_remaining = self.samples_remaining.saturating_sub(1);

        // Wrap position to [0.0, 1.0] range (loop through file)
        if self.position < 0.0 {
            self.position += 1.0;
        } else if self.position > 1.0 {
            self.position -= 1.0;
        }

        // Deactivate grain when we played through the whole grain
        if self.samples_remaining == 0 {
            self.active = false;
        }

        let envelope = envelope_value * self.volume;
        let panning = self.panning;

        GrainOutput {
            envelope,
            panning,
            position,
        }
    }
}
