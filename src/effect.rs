use std::any::Any;

use four_cc::FourCC;

use crate::{parameter::ParameterValueUpdate, ClonableParameter, Error, SourceTime};

// -------------------------------------------------------------------------------------------------

pub mod chorus;
pub mod compressor;
pub mod dcfilter;
pub mod distortion;
pub mod eq5;
pub mod filter;
pub mod gain;
pub mod reverb;

// -------------------------------------------------------------------------------------------------

/// Carries [`Effect`] specific payloads/automation, which can't or should not be expressed as
/// [`Parameter`](crate::Parameter).
///
/// This trait is implemented by message enums specific to each effect. It provides a way to
/// identify the target effect and access the message payload as a `dyn Any`, which can then be
/// downcast to the concrete message type within the effect's `process_message` implementation.
///
/// Messages are always applied in the effect's DSP real-time thread.
pub trait EffectMessage: Any + Send + Sync {
    /// The static name of the target effect for this message.
    ///
    /// This should match the `name()` of the target `Effect` implementation. It is used by the
    /// `Player` to prevent sending messages to the wrong effect type.
    fn effect_name(&self) -> &'static str;

    /// Returns the message payload as a `dyn Any` reference.
    ///
    /// This allows the effect to downcast the payload to its specific message enum type.
    fn payload(&self) -> &dyn Any;
}

// -------------------------------------------------------------------------------------------------

/// Type used in [`Effect::process_message`] to receive messages.
///
/// It allows for dynamic dispatch to different message types.
pub type EffectMessagePayload = dyn EffectMessage;

// -------------------------------------------------------------------------------------------------

/// Frame and wall-clock time reference for an audio effect's process function.
///
/// TODO: should be a custom time struct with bpm, beat positions and stuff
pub type EffectTime = SourceTime;

// -------------------------------------------------------------------------------------------------

/// Effects manipulate audio samples in `f32` format and can be `Send` and `Sync`ed across threads.
///
/// Buffers are processed in-place. Effect parameters can be changed at runtime by sending
/// [`EffectMessage`]s via the player's [`send_effect_message`](crate::Player::send_effect_message).
///
/// NB: `process` and `process_message` are called in realtime audio threads, so they must not
/// block! All other functions are called in the main thread to initialize the effect.
pub trait Effect: Send + Sync + 'static {
    /// A unique, static name for the effect.
    ///
    /// This name is used to associate `EffectMessage`s with their target effect type, preventing
    /// mis-typed messages from being processed. It can also be used for logging or in UIs.
    fn name(&self) -> &'static str;

    /// Returns a list of parameter descriptors for this effect.
    ///
    /// This can be used by UIs or automation systems to query available parameters of a specific
    /// effect. This method may only be called on non-real-time threads: Usually it will be called
    /// after creating a new effect instance, before adding it to the player's effect chains, in
    /// order to gather parameter info for generic effect UIs.  
    fn parameters(&self) -> Vec<&dyn ClonableParameter>;

    /// Initializes the effect with the audio output's properties.
    ///
    /// This method is called once by the `Player` before the effect is used. It runs on a
    /// non-real-time thread, so it's safe to perform allocations (e.g., for delay buffers) or
    /// other setup tasks.
    ///
    /// If an error is returned, the effect will not be added to the mixer.
    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        max_frames: usize,
    ) -> Result<(), Error>;

    /// Processes an audio buffer in-place, applying the effect.
    ///
    /// This method is called repeatedly on the real-time audio thread. To avoid audio glitches,
    /// it must not block, allocate memory, or perform other time-consuming operations.
    ///
    /// Use [`InterleavedBufferMut`](crate::utils::InterleavedBufferMut) to get channel/frame
    /// representations of the given output buffer as needed.
    fn process(&mut self, output: &mut [f32], time: &EffectTime);

    /// Handles a parameter update sent to the effect.
    ///
    /// This method is called on the real-time audio thread when a parameter change is scheduled
    /// for processing. The implementation should match on the `id` and update its internal
    /// state accordingly by using the `value` which can be a raw or normalized value.
    ///
    /// Like `process`, this method must not block, allocate memory, or perform other
    /// time-consuming operations.
    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error>;

    /// Handles optional effect specific messages sent to the effect. This can be used to pass effect
    /// specific non parameter changes to the effect processor.
    ///
    /// This method is called on the real-time audio thread when a message is scheduled for processing.
    /// The implementation should downcast the `message` payload to its specific message enum type
    /// and update its internal state accordingly.
    ///
    /// Like `process`, this method must not block, allocate memory, or perform other
    /// time-consuming operations.
    fn process_message(&mut self, _message: &EffectMessagePayload) -> Result<(), Error> {
        Err(Error::ParameterError(format!(
            "{}: Received unexpected message payload.",
            self.name()
        )))
    }
}
