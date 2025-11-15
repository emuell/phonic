use std::sync::Arc;

use crossbeam_queue::ArrayQueue;

use crate::utils::smoothing::{apply_smoothed_gain, ExponentialSmoothedValue, SmoothedValue};

use super::{Source, SourceTime};

// -------------------------------------------------------------------------------------------------

/// Messages to control the amplified source
pub enum AmplifiedSourceMessage {
    SetVolume(f32), // new volume value
}

// -------------------------------------------------------------------------------------------------

/// A source which applies a volume factor to some other source's output
pub struct AmplifiedSource<InputSource: Source + 'static> {
    source: InputSource,
    volume: ExponentialSmoothedValue,
    message_queue: Arc<ArrayQueue<AmplifiedSourceMessage>>,
}

impl<InputSource: Source + 'static> AmplifiedSource<InputSource> {
    pub fn new(source: InputSource, volume: f32) -> Self
    where
        InputSource: Source,
    {
        debug_assert!(volume >= 0.0, "Invalid volume factor");
        let volume = ExponentialSmoothedValue::new(volume, source.sample_rate());

        // we're expecting a single message only, as events are already scheduled by the mixer
        const MESSAGE_QUEUE_SIZE: usize = 1;
        let message_queue = Arc::new(ArrayQueue::new(MESSAGE_QUEUE_SIZE));

        Self {
            source,
            volume,
            message_queue,
        }
    }

    /// Returns the message queue for this source.
    pub fn message_queue(&self) -> Arc<ArrayQueue<AmplifiedSourceMessage>> {
        self.message_queue.clone()
    }

    /// Internal message handling
    fn process_messages(&mut self) {
        while let Some(msg) = self.message_queue.pop() {
            match msg {
                AmplifiedSourceMessage::SetVolume(volume) => {
                    self.volume.set_target(volume);
                }
            }
        }
    }
}

impl<InputSource: Source + 'static> Source for AmplifiedSource<InputSource> {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // process pending messages
        self.process_messages();

        // write input source
        let written = self.source.write(output, time);

        // apply volume using helper
        let written_out = &mut output[0..written];
        apply_smoothed_gain(written_out, &mut self.volume);
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
