use std::any::Any;

use crate::{Error, SourceTime};

// -------------------------------------------------------------------------------------------------

/// TODO: should be a custom time struct with bpm, beat positions and stuff
pub type EffectTime = SourceTime;

// -------------------------------------------------------------------------------------------------

/// Message send to the effect to e.g. change effect parameters.
pub type EffectMessage = dyn Any + Send + Sync;

// -------------------------------------------------------------------------------------------------

/// Audio effect source which applies a DSP effect to an output buffer.
pub trait Effect: Send + Sync {
    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        max_frames: usize,
    ) -> Result<(), Error>;

    fn process(&mut self, output: &mut [f32], time: &EffectTime);
    fn process_message(&mut self, message: &EffectMessage);
}
