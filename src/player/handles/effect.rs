use std::sync::Arc;

use crossbeam_queue::ArrayQueue;
use dashmap::DashMap;
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
    mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
    collector_handle: Handle,
}

impl EffectHandle {
    pub(crate) fn new(
        effect_id: EffectId,
        mixer_id: MixerId,
        effect_name: &'static str,
        mixer_event_queues: Arc<DashMap<MixerId, Arc<ArrayQueue<MixerMessage>>>>,
        collector_handle: Handle,
    ) -> Self {
        Self {
            effect_id,
            mixer_id,
            effect_name,
            mixer_event_queues,
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

    /// Set a raw parameter value on an effect at a specific sample time or immediately.
    ///
    /// The `value` must be of the correct type for the parameter: `f32`, `i32`, `bool`,
    /// or the specific enum type used by the parameter.
    pub fn set_parameter<V: std::any::Any + Send + Sync, T: Into<Option<u64>>>(
        &self,
        parameter_id: FourCC,
        value: V,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into().unwrap_or(0);
        let value = Owned::new(
            &self.collector_handle,
            ParameterValueUpdate::Raw(Box::new(value)),
        );

        if self
            .mixer_event_queue()?
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

    /// Set a normalized parameter value at a specific sample time or immediately.
    ///
    /// The `value` must be in the range `0.0..=1.0`.
    pub fn set_parameter_normalized<T: Into<Option<u64>>>(
        &self,
        parameter_id: FourCC,
        value: f32,
        sample_time: T,
    ) -> Result<(), Error> {
        let sample_time = sample_time.into().unwrap_or(0);
        let value = Owned::new(
            &self.collector_handle,
            ParameterValueUpdate::Normalized(value),
        );

        if self
            .mixer_event_queue()?
            .push(MixerMessage::ProcessEffectParameterUpdate {
                effect_id: self.effect_id,
                parameter_id,
                value,
                sample_time,
            })
            .is_err()
        {
            Err(Self::mixer_event_queue_error("set_parameter_normalized"))
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
            .mixer_event_queue()?
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

    fn mixer_event_queue(&self) -> Result<Arc<ArrayQueue<MixerMessage>>, Error> {
        Ok(Arc::clone(
            self.mixer_event_queues
                .get(&self.mixer_id)
                .ok_or(Error::MixerNotFoundError(self.mixer_id))?
                .value(),
        ))
    }

    fn mixer_event_queue_error(event_name: &str) -> Error {
        log::warn!("Mixer's event queue is full. Failed to send a {event_name} event.");
        log::warn!("Increase the mixer event queue to prevent this from happening...");
        Error::SendError("Mixer queue is full".to_string())
    }
}
