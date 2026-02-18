use std::sync::{mpsc::SyncSender, Arc};

use crossbeam_queue::ArrayQueue;

use crate::{
    generator::{Generator, GeneratorPlaybackMessage, GeneratorPlaybackOptions},
    source::{unique_source_id, Source, SourceTime},
    PlaybackId, PlaybackStatusEvent,
};

// -------------------------------------------------------------------------------------------------

/// A generator source which does not produce any samples and ignores all events.
///
/// Can be useful when a temporary placeholder generator is needed.
#[derive(Debug, Clone)]
pub struct EmptyGenerator {
    is_transient: bool,
    playback_id: PlaybackId,
    playback_options: GeneratorPlaybackOptions,
    playback_message_queue: Arc<ArrayQueue<GeneratorPlaybackMessage>>,
    playback_status_sender: Option<SyncSender<PlaybackStatusEvent>>,
    channel_count: usize,
    sample_rate: u32,
}

impl EmptyGenerator {
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        let is_transient = false;
        let playback_id = unique_source_id();
        let playback_message_queue = Arc::new(ArrayQueue::new(16));
        let playback_options = GeneratorPlaybackOptions::default();
        let playback_status_sender = None;
        Self {
            is_transient,
            playback_id,
            playback_options,
            playback_message_queue,
            playback_status_sender,
            channel_count,
            sample_rate,
        }
    }
}

impl Default for EmptyGenerator {
    fn default() -> Self {
        Self::new(2, 44100)
    }
}

impl Source for EmptyGenerator {
    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.is_transient
    }

    fn weight(&self) -> usize {
        0
    }

    fn write(&mut self, _output: &mut [f32], _time: &SourceTime) -> usize {
        while self.playback_message_queue.pop().is_some() {
            // consume, but ignore events
        }
        0
    }
}

impl Generator for EmptyGenerator {
    fn generator_name(&self) -> String {
        "void".to_string()
    }

    fn playback_id(&self) -> PlaybackId {
        self.playback_id
    }

    fn playback_options(&self) -> &GeneratorPlaybackOptions {
        &self.playback_options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<GeneratorPlaybackMessage>> {
        Arc::clone(&self.playback_message_queue)
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.playback_status_sender.clone()
    }

    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.playback_status_sender = sender;
    }

    fn is_transient(&self) -> bool {
        self.is_transient
    }

    fn set_is_transient(&mut self, is_transient: bool) {
        self.is_transient = is_transient;
    }
}
