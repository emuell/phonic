use std::any::Any;

use crate::{Error, SourceTime};

// -------------------------------------------------------------------------------------------------

pub mod chorus;
pub mod dcfilter;
pub mod filter;
pub mod reverb;

// -------------------------------------------------------------------------------------------------

/// TODO: should be a custom time struct with bpm, beat positions and stuff
pub type EffectTime = SourceTime;

// -------------------------------------------------------------------------------------------------

/// Message send to the effect in audio time to e.g. change effect parameters.
pub type EffectMessage = dyn Any + Send + Sync;

// -------------------------------------------------------------------------------------------------

/// Audio effect source which applies a DSP effect to an output buffer.
pub trait Effect: Send + Sync + 'static {
    /// Initialize the effect after construction. This is always called from a non realtime thread,
    /// so effects can (pre)allocate memory here and do other things to initialize audio processing.
    ///
    /// When the effect returns an error here (e.g. for unsupported channel layouts), it will not run.
    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        max_frames: usize,
    ) -> Result<(), Error>;

    /// Process the effect in-place to the given audio buffer.
    /// This is always **called from a real-time thread**, so don't block or allocate heap memory here.
    fn process(&mut self, output: &mut [f32], time: &EffectTime);

    /// Handle Effect specific messages.  
    /// This is always **called from a real-time thread**, so don't block or allocate heap memory here.
    fn process_message(&mut self, message: &EffectMessage);
}
