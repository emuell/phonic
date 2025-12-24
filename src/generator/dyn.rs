use std::sync::{mpsc::SyncSender, Arc};

use crossbeam_queue::ArrayQueue;
use four_cc::FourCC;

use crate::{
    Error, Generator, GeneratorPlaybackMessage, GeneratorPlaybackOptions, Parameter,
    ParameterValueUpdate, PlaybackId, PlaybackStatusEvent, Source, SourceTime,
};

// -------------------------------------------------------------------------------------------------

/// A generator impl which wraps a (boxed) `dyn Generator`.
///
/// Allows playing dyn generator sources via
/// [Player::play_generator_source](crate::Player::play_generator_source).
pub struct DynGenerator {
    generator: Box<dyn Generator>,
}

impl DynGenerator {
    pub fn new(generator: Box<dyn Generator>) -> Self {
        Self { generator }
    }
}

impl Source for DynGenerator {
    fn channel_count(&self) -> usize {
        self.generator.channel_count()
    }

    fn is_exhausted(&self) -> bool {
        self.generator.is_exhausted()
    }

    fn sample_rate(&self) -> u32 {
        self.generator.sample_rate()
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        self.generator.write(output, time)
    }
}

impl Generator for DynGenerator {
    fn generator_name(&self) -> String {
        self.generator.generator_name()
    }

    fn parameters(&self) -> Vec<&dyn Parameter> {
        self.generator.parameters()
    }

    fn playback_id(&self) -> PlaybackId {
        self.generator.playback_id()
    }

    fn playback_options(&self) -> &GeneratorPlaybackOptions {
        self.generator.playback_options()
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        self.generator.playback_message_queue()
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.generator.playback_status_sender()
    }

    fn process_parameter_update(
        &mut self,
        id: FourCC,
        value: &ParameterValueUpdate,
    ) -> Result<(), Error> {
        self.generator.process_parameter_update(id, value)
    }

    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.generator.set_playback_status_sender(sender);
    }
}
