use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    effect::EffectMessage,
    error::Error,
    parameter::ParameterValueUpdate,
    player::{EffectId, MixerId},
    source::mixed::MixerMessage,
};
use basedrop::{Handle, Owned};

// -------------------------------------------------------------------------------------------------

/// Automate [`Effect`](crate::Effect) parameters or send messages.
///
/// Handles are `Send` and `Sync` so they can be sent across threads.
#[derive(Clone)]
pub struct EffectHandle {
    effect_id: EffectId,
    mixer_id: MixerId,
    effect_name: &'static str,
    mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
    collector_handle: Handle,
}

impl EffectHandle {
    pub(crate) fn new(
        effect_id: EffectId,
        mixer_id: MixerId,
        effect_name: &'static str,
        mixer_event_queue: Arc<ArrayQueue<MixerMessage>>,
        collector_handle: Handle,
    ) -> Self {
        Self {
            effect_id,
            mixer_id,
            effect_name,
            mixer_event_queue,
            collector_handle,
        }
    }

    /// Get the effect ID.
    pub fn id(&self) -> EffectId {
        self.effect_id
    }

    /// Get the mixer ID this effect belongs to.
    pub fn mixer_id(&self) -> MixerId {
        self.mixer_id
    }

    /// Get the effect's name.
    pub fn effect_name(&self) -> &'static str {
        self.effect_name
    }

    /// Set a parameter's value via the given raw or normalized value update definition
    /// at a specific sample time or immediately.
    ///
    /// Note: Value update (id, value) tuples can be created safely via `value_update` functions
    /// in [FloatParameter](crate::parameters::FloatParameter), [IntegerParameter](crate::parameters::IntegerParameter),
    /// [EnumParameter](crate::parameters::EnumParameter) and [BooleanParameter](crate::parameters::BooleanParameter).
    pub fn set_parameter<T: Into<Option<u64>>>(
        &self,
        (parameter_id, update): (FourCC, ParameterValueUpdate),
        sample_time: T,
    ) -> Result<(), Error> {
        if let ParameterValueUpdate::Normalized(normalized_value) = update {
            if !(0.0..=1.0).contains(&normalized_value) {
                return Err(Error::ParameterError(format!(
                    "Invalid parameter update: value should be a normalized value, but is: '{normalized_value}'"
                )));
            }
        }
        let sample_time = sample_time.into().unwrap_or(0);
        let value = Owned::new(&self.collector_handle, update);
        if self
            .mixer_event_queue
            .push(MixerMessage::ProcessEffectParameterUpdate {
                effect_id: self.effect_id,
                parameter_id,
                value,
                sample_time,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("set_parameter"))
        } else {
            Ok(())
        }
    }

    /// Set multple parameter values via the given raw or normalized value update definition
    /// at a specific sample time or immediately.
    ///
    /// Note: Value update (id, value) tuples can be created safely via `value_update` functions
    /// in [FloatParameter](crate::parameters::FloatParameter), [IntegerParameter](crate::parameters::IntegerParameter),
    /// [EnumParameter](crate::parameters::EnumParameter) and [BooleanParameter](crate::parameters::BooleanParameter).
    pub fn set_parameters<T: Into<Option<u64>>>(
        &self,
        values: Vec<(FourCC, ParameterValueUpdate)>,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into().unwrap_or(0);
        let values = Owned::new(&self.collector_handle, values);
        if self
            .mixer_event_queue
            .push(MixerMessage::ProcessEffectParameterUpdates {
                effect_id: self.effect_id,
                values,
                sample_time,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("set_parameter"))
        } else {
            Ok(())
        }
    }

    /// Send a custom message to the effect at a specific sample time or immediately.
    pub fn send_message<M: EffectMessage + 'static, T: Into<Option<u64>>>(
        &self,
        message: M,
        sample_time: T,
    ) -> Result<(), Error> {
        // Verify message matches effect name
        if message.effect_name() != self.effect_name {
            return Err(Error::ParameterError(format!(
                "Invalid message: Trying to send a `{}` message to effect '{}' (id: {})",
                message.effect_name(),
                self.effect_name,
                self.effect_id
            )));
        }

        let message = Owned::new(
            &self.collector_handle,
            Box::new(message) as Box<dyn EffectMessage>,
        );
        let sample_time = sample_time.into().unwrap_or(0);

        if self
            .mixer_event_queue
            .push(MixerMessage::ProcessEffectMessage {
                effect_id: self.effect_id,
                message,
                sample_time,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("send_message"))
        } else {
            Ok(())
        }
    }

    fn mixer_event_queue_error(event_name: &str) -> Error {
        log::warn!("Mixer's event queue is full. Failed to send a {event_name} event.");
        log::warn!("Increase the mixer event queue to prevent this from happening...");
        Error::SendError("Mixer queue is full".to_string())
    }
}
