use std::sync::Arc;

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use sort::bubble_sort_cmp;

use crate::{
    player::PlaybackMessageSender,
    source::{Source, SourceTime},
    PlaybackId,
};

// -------------------------------------------------------------------------------------------------

/// Mixer internal struct to keep track of currently playing sources.
struct MixedPlayingSource {
    is_active: bool,
    playback_id: PlaybackId,
    playback_message_queue: PlaybackMessageSender,
    source: Owned<Box<dyn Source>>,
    start_time: u64,
    stop_time: Option<u64>,
}

// -------------------------------------------------------------------------------------------------

/// Messages send from player to mixer to start or stop playing sources.
pub enum MixedSourceMsg {
    AddSource {
        playback_id: PlaybackId,
        playback_message_queue: PlaybackMessageSender,
        source: Owned<Box<dyn Source>>,
        sample_time: u64,
    },
    StopSource {
        playback_id: PlaybackId,
        sample_time: u64,
    },
    #[allow(dead_code)]
    RemoveAllSources,
    RemoveAllPendingSources,
}

// -------------------------------------------------------------------------------------------------

/// A [`Source`] which converts and mixes other sources together.
pub struct MixedSource {
    playing_sources: Vec<MixedPlayingSource>,
    event_queue: Arc<ArrayQueue<MixedSourceMsg>>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    /// Create a new mixer source with the given signal specs.
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        // avoid allocs in real-time threads
        const PLAYING_EVENTS_CAPACITY: usize = 1024;
        let playing_sources = Vec::with_capacity(PLAYING_EVENTS_CAPACITY);

        // assume that we'll never start/stop more than 4096 samples per write batch
        const EVENT_QUEUE_SIZE: usize = 4096;
        let event_queue = Arc::new(ArrayQueue::new(EVENT_QUEUE_SIZE));

        // temp mix buffer size
        const BUFFER_SIZE: usize = 8 * 1024;
        let temp_out = vec![0.0; BUFFER_SIZE];

        Self {
            playing_sources,
            event_queue,
            channel_count,
            sample_rate,
            temp_out,
        }
    }

    /// Allows controlling the mixer by pushing messages into this event queue.
    /// NB: When adding new sources, ensure they match the mixers sample rate and channel layout
    pub(crate) fn event_queue(&self) -> Arc<ArrayQueue<MixedSourceMsg>> {
        self.event_queue.clone()
    }

    /// remove all entries from self.playing_sources which match the given filter function.
    fn remove_matching_sources<F>(&mut self, match_fn: F)
    where
        F: Fn(&MixedPlayingSource) -> bool,
    {
        self.playing_sources.retain(move |p| !match_fn(p));
    }

    /// remove all entries from self.playing_sources.
    fn remove_all_sources(&mut self) {
        self.playing_sources.clear();
    }

    /// Process pending mixer events
    fn process_events(&mut self, time: &SourceTime) {
        let mut sources_added = false;
        while let Some(event) = self.event_queue.pop() {
            match event {
                MixedSourceMsg::AddSource {
                    playback_id,
                    playback_message_queue,
                    source,
                    sample_time,
                } => {
                    debug_assert_eq!(
                        source.channel_count(),
                        self.channel_count,
                        "adjust source's channel layout before adding it"
                    );
                    debug_assert_eq!(
                        source.sample_rate(),
                        self.sample_rate,
                        "adjust source's sample rate before adding it"
                    );
                    sources_added = true;
                    self.playing_sources.push(MixedPlayingSource {
                        is_active: true,
                        playback_id,
                        playback_message_queue,
                        source,
                        start_time: sample_time,
                        stop_time: None,
                    });
                }
                MixedSourceMsg::StopSource {
                    playback_id,
                    sample_time,
                } => {
                    if let Some(source) = self
                        .playing_sources
                        .iter_mut()
                        .find(|s| s.playback_id == playback_id)
                    {
                        source.stop_time = Some(sample_time);
                    }
                }
                MixedSourceMsg::RemoveAllPendingSources => {
                    // remove all sources which are not yet playing
                    self.remove_matching_sources(|source| source.start_time > time.pos_in_frames);
                }
                MixedSourceMsg::RemoveAllSources => {
                    self.remove_all_sources();
                }
            }
        }

        // Sort sources by start time if any new sources were added
        if sources_added {
            // keep sources sorted by sample time: this makes batch processing easier
            // NB: use "swap" based sorting here to avoid memory allocations
            bubble_sort_cmp(&mut self.playing_sources, |a, b| {
                a.start_time.cmp(&b.start_time) as isize
            });
        }
    }
}

impl Source for MixedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // Process all pending events
        self.process_events(time);

        // Return early if no active sources
        if self.playing_sources.is_empty() {
            return 0;
        }
        // clear entire output first, as we're only adding below
        output.fill(0.0);

        // run and add all playing sources
        let output_frame_count = output.len() / self.channel_count;
        'all_sources: for playing_source in self.playing_sources.iter_mut() {
            let mut total_written = 0;

            // apply source's sample start time
            if playing_source.start_time > time.pos_in_frames {
                let frames_until_source_starts =
                    (playing_source.start_time - time.pos_in_frames) as usize;
                if frames_until_source_starts > 0 {
                    if frames_until_source_starts >= output_frame_count {
                        // playing_sources are sorted by sample time: all following sources will run
                        // after this source, and thus also can also be skipped...
                        break 'all_sources;
                    }
                    // move offset to the sample's start pos
                    total_written += frames_until_source_starts * self.channel_count;
                }
            }

            // run and mix down the source
            let source = &mut playing_source.source;
            'source: while total_written < output.len() {
                let source_time =
                    time.with_added_frames((total_written / self.channel_count) as u64);

                // check if there's a pending stop command for the source
                let mut samples_until_stop = u64::MAX;
                if let Some(stop_time_in_frames) = playing_source.stop_time {
                    if stop_time_in_frames >= source_time.pos_in_frames {
                        samples_until_stop = (stop_time_in_frames - source_time.pos_in_frames)
                            * self.channel_count as u64;
                    }
                }
                if samples_until_stop == 0 {
                    let sender = &playing_source.playback_message_queue;
                    if let Err(err) = sender.send_stop() {
                        log::warn!("failed to send stop event: {}", err)
                    }
                    samples_until_stop = u64::MAX;
                }

                // run source on temp_out until we've filled up the whole final output
                let remaining = (output.len() - total_written).min(samples_until_stop as usize);
                let to_write = remaining.min(self.temp_out.len());
                let written = source.write(&mut self.temp_out[..to_write], &source_time);

                // add output of the source to the final output
                let remaining_out = &mut output[total_written..];
                let written_out = &self.temp_out[..written];
                for (o, i) in remaining_out.iter_mut().zip(written_out) {
                    *o += *i;
                }
                total_written += written;

                // stop processing sources which are now exhausted
                if source.is_exhausted() {
                    playing_source.is_active = false;
                    break 'source;
                }
            }
        }

        // drop all sources which finished playing in this iteration
        self.remove_matching_sources(|s| !s.is_active);

        // return output len as we've cleared the entire output before processing
        output.len()
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        // mixer never is exhausted, as we may get new sources added
        false
    }
}
