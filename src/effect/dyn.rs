use four_cc::FourCC;

use crate::{Effect, EffectMessagePayload, EffectTime, Error, Parameter, ParameterValueUpdate};

// -------------------------------------------------------------------------------------------------

/// A Effect impl which wraps a (boxed) `dyn Effect`.
///
/// Allows adding dyn effects via [Player::add_effect](crate::Player::add_effect).
pub struct DynEffect {
    effect: Box<dyn Effect>,
}

impl DynEffect {
    pub fn new(effect: Box<dyn Effect>) -> Self {
        Self { effect }
    }
}

impl Effect for DynEffect {
    fn name(&self) -> &'static str {
        self.effect.name()
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        self.effect.parameters()
    }

    fn initialize(
        &mut self,
        sample_rate: u32,
        channel_count: usize,
        max_frames: usize,
    ) -> Result<(), Error> {
        self.effect
            .initialize(sample_rate, channel_count, max_frames)
    }

    fn process_started(&mut self) {
        self.effect.process_started()
    }

    fn process_stopped(&mut self) {
        self.effect.process_stopped()
    }

    fn process(&mut self, output: &mut [f32], time: &EffectTime) {
        self.effect.process(output, time)
    }

    fn process_tail(&self) -> Option<usize> {
        self.effect.process_tail()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        self.effect.process_parameter_update(id, value)
    }

    fn process_message(&mut self, message: &EffectMessagePayload) -> Result<(), Error> {
        self.effect.process_message(message)
    }
}
