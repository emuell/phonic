use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

use crate::utils::smoothing::{apply_smoothed_panning, ExponentialSmoothedValue, SmoothedValue};

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// Messages to control the panned source
pub enum PannedSourceMessage {
    SetPanning(f32), // new panning value
}

// -------------------------------------------------------------------------------------------------

/// A source which applies a pan factor to some other source's output
pub struct PannedSource {
    source: Box<dyn Source>,
    panning: ExponentialSmoothedValue,
    message_queue: Arc<ArrayQueue<PannedSourceMessage>>,
}

impl PannedSource {
    pub fn new<InputSource>(source: InputSource, panning: f32) -> Self
    where
        InputSource: Source,
    {
        debug_assert!((-1.0..=1.0).contains(&panning), "Invalid panning factor");
        let smoothed_panning = ExponentialSmoothedValue::new(panning, source.sample_rate());

        // we're expecting a single message only, as events are already scheduled by the mixer
        const MESSAGE_QUEUE_SIZE: usize = 1;

        Self {
            source: Box::new(source),
            panning: smoothed_panning,
            message_queue: Arc::new(ArrayQueue::new(MESSAGE_QUEUE_SIZE)),
        }
    }

    /// Returns the message queue for this source.
    pub fn message_queue(&self) -> Arc<ArrayQueue<PannedSourceMessage>> {
        self.message_queue.clone()
    }

    /// Internal event handling
    fn process_messages(&mut self) {
        while let Some(msg) = self.message_queue.pop() {
            match msg {
                PannedSourceMessage::SetPanning(panning) => {
                    self.panning.set_target(panning);
                }
            }
        }
    }
}

impl Source for PannedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // process pending messages
        self.process_messages();

        // write input source
        let written = self.source.write(output, time);

        // apply panning
        let channel_count = self.source.channel_count();
        apply_smoothed_panning(&mut output[..written], channel_count, &mut self.panning);
        written
    }

    fn channel_count(&self) -> usize {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn is_exhausted(&self) -> bool {
        self.source.is_exhausted()
    }
}
