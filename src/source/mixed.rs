use std::{collections::VecDeque, sync::Arc};

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use sort::bubble_sort_cmp;

use crate::{
    player::PlaybackMessageSender,
    source::{file::FilePlaybackMessage, Source, SourceTime},
    utils::buffer::{add_buffers, clear_buffer},
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

/// Mixer internal event to update playing source playback properties.
struct MixedSourceEvent {
    playback_id: PlaybackId,
    speed: f64,
    glide: Option<f32>, // semitones per second
    sample_time: u64,
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
    SetSpeed {
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>, // semitones per second
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
    source_events: VecDeque<MixedSourceEvent>,
    event_queue: Arc<ArrayQueue<MixedSourceMsg>>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    /// Create a new mixer source with the given signal specs.
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        // prealloc event queues
        const PLAYING_EVENTS_CAPACITY: usize = 1024;
        let playing_sources = Vec::with_capacity(PLAYING_EVENTS_CAPACITY);

        const SOURCE_EVENTS_CAPACITY: usize = 4096;
        let source_events = VecDeque::with_capacity(SOURCE_EVENTS_CAPACITY);

        const EVENT_QUEUE_SIZE: usize = 4096;
        let event_queue = Arc::new(ArrayQueue::new(EVENT_QUEUE_SIZE));

        // create temp mix buffer
        const BUFFER_SIZE: usize = 8 * 1024;
        let temp_out = vec![0.0; BUFFER_SIZE];

        Self {
            playing_sources,
            source_events,
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

    /// remove all entries from self.source_events which match the given filter function.
    fn remove_matching_source_events<F>(&mut self, match_fn: F)
    where
        F: Fn(&MixedSourceEvent) -> bool,
    {
        self.source_events.retain(move |p| !match_fn(p));
    }

    /// remove all entries from self.playing_sources and flush all pending events.
    fn remove_all_playing_sources(&mut self) {
        self.playing_sources.clear();
        self.source_events.clear();
    }

    /// Process pending mixer events
    fn process_playback_events(&mut self, time: &SourceTime) {
        let mut sources_added = false;
        let mut source_events_added = false;
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
                MixedSourceMsg::SetSpeed {
                    playback_id,
                    speed,
                    glide,
                    sample_time,
                } => {
                    self.source_events.push_back(MixedSourceEvent {
                        playback_id,
                        speed,
                        glide,
                        sample_time,
                    });
                    source_events_added = true;
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
                    self.remove_matching_source_events(|source| {
                        source.sample_time > time.pos_in_frames
                    });
                }
                MixedSourceMsg::RemoveAllSources => {
                    self.remove_all_playing_sources();
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

        // Sort events by sample time if any new events were added. see also sources_added...
        if source_events_added {
            bubble_sort_cmp(self.source_events.make_contiguous(), |a, b| {
                a.sample_time.cmp(&b.sample_time) as isize
            });
        }
    }

    // Process pending mixer source events that are due at the current time
    fn process_source_events(&mut self, current_time: u64) {
        while self
            .source_events
            .front()
            .is_some_and(|e| e.sample_time <= current_time)
        {
            let event = self.source_events.pop_front().unwrap();
            if let Some(source) = self
                .playing_sources
                .iter()
                .find(|s| s.playback_id == event.playback_id)
            {
                if let PlaybackMessageSender::File(queue) = &source.playback_message_queue {
                    if queue
                        .push(FilePlaybackMessage::SetSpeed(event.speed, event.glide))
                        .is_err()
                    {
                        log::warn!("failed to send set speed event");
                    }
                }
            }
        }
    }

    // Write and mix down all playing sources into the given buffer at the given time.
    fn write_sources(&mut self, output: &mut [f32], time: SourceTime) {
        let output_frame_count = output.len() / self.channel_count();
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
                        log::warn!("failed to send stop event: {err}")
                    }
                    samples_until_stop = u64::MAX;
                }

                // run source on temp_out until we've filled up the whole slice
                let remaining = (output.len() - total_written).min(samples_until_stop as usize);
                let to_write = remaining.min(self.temp_out.len());
                let written = source.write(&mut self.temp_out[..to_write], &source_time);

                // add output of the source to the final output slice
                let remaining_out = &mut output[total_written..total_written + written];
                let written_out = &self.temp_out[..written];
                add_buffers(remaining_out, written_out);
                total_written += written;

                // stop processing sources which are now exhausted
                if source.is_exhausted() {
                    playing_source.is_active = false;
                    break 'source;
                }
            }
        }
    }
}

impl Source for MixedSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // Process all pending events
        self.process_playback_events(time);

        // Return early if no active sources
        if self.playing_sources.is_empty() {
            return 0;
        }

        // clear entire output first, as we're only adding below
        clear_buffer(output);

        let output_frame_count = output.len() / self.channel_count;
        let mut total_frames_written = 0;

        while total_frames_written < output_frame_count {
            let current_time_in_frames = time.pos_in_frames + total_frames_written as u64;

            // process pending source events
            self.process_source_events(current_time_in_frames);

            // determine how many frames to process
            let frames_to_process = {
                let frames_remaining_in_output = output_frame_count - total_frames_written;
                if let Some(event) = self.source_events.front() {
                    let samples_to_next_event =
                        (event.sample_time - current_time_in_frames) as usize;
                    frames_remaining_in_output.min(samples_to_next_event)
                } else {
                    frames_remaining_in_output
                }
            };

            // process next chunk
            if frames_to_process > 0 {
                let chunk_time = time.with_added_frames(total_frames_written as u64);
                let chunk_output = &mut output[total_frames_written * self.channel_count
                    ..(total_frames_written + frames_to_process) * self.channel_count];

                self.write_sources(chunk_output, chunk_time);
                total_frames_written += frames_to_process;
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
