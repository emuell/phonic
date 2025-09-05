use std::{collections::VecDeque, sync::Arc};

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;
use sort::bubble_sort_cmp;

use crate::{
    effect::{Effect, EffectMessage},
    player::{EffectId, PlaybackMessageSender},
    source::{file::FilePlaybackMessage, Source, SourceTime},
    utils::buffer::{add_buffers, clear_buffer},
    PlaybackId,
};

// -------------------------------------------------------------------------------------------------

/// Mixer internal struct to keep track of currently playing sources.
struct PlayingSource {
    is_active: bool,
    playback_id: PlaybackId,
    playback_message_queue: PlaybackMessageSender,
    source: Owned<Box<dyn Source>>,
    start_time: u64,
    stop_time: Option<u64>,
}

// -------------------------------------------------------------------------------------------------

/// Mixer internal event to schedule playback changes.
enum MixerEvent {
    SetFileSourceSpeed {
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: u64,
    },
    ProcessEffectMessage {
        effect_id: EffectId,
        message: Owned<Box<dyn EffectMessage>>,
        sample_time: u64,
    },
}

impl MixerEvent {
    fn sample_time(&self) -> u64 {
        match self {
            Self::SetFileSourceSpeed { sample_time, .. } => *sample_time,
            Self::ProcessEffectMessage { sample_time, .. } => *sample_time,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Messages send from player to mixer to start or stop playing sources.
pub(crate) enum MixerSourceMessage {
    AddSource {
        playback_id: PlaybackId,
        playback_message_queue: PlaybackMessageSender,
        source: Owned<Box<dyn Source>>,
        sample_time: u64,
    },
    AddEffect {
        id: EffectId,
        effect: Box<dyn Effect>,
    },
    AddMixer {
        id: EffectId,
        mixer: Box<MixedSource>,
    },
    ProcessEffectMessage {
        effect_id: EffectId,
        message: Owned<Box<dyn EffectMessage>>,
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
    playing_sources: Vec<PlayingSource>,
    mixers: Vec<(EffectId, Box<MixedSource>)>,
    effects: Vec<(EffectId, Box<dyn Effect>)>,
    message_queue: Arc<ArrayQueue<MixerSourceMessage>>,
    events: VecDeque<MixerEvent>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    /// The size of the temporary buffer used for mixing, in samples.
    /// Sources and Effects will never requested to produce more samples than this const.
    pub const MAX_MIX_BUFFER_SAMPLES: usize = 8 * 1024;

    /// Create a new mixer source with the given signal specs.
    pub fn new(channel_count: usize, sample_rate: u32) -> Self {
        // prealloc playing source, sub mixer and effect lists
        const PLAYING_EVENTS_CAPACITY: usize = 1024;
        let playing_sources = Vec::with_capacity(PLAYING_EVENTS_CAPACITY);
        const MIXERS_CAPACITY: usize = 16;
        let mixers = Vec::with_capacity(MIXERS_CAPACITY);
        const EFFECTS_CAPACITY: usize = 16;
        let effects = Vec::with_capacity(EFFECTS_CAPACITY);

        // prealloc event queues
        const MESSAGE_QUEUE_SIZE: usize = 4096;
        let message_queue = Arc::new(ArrayQueue::new(MESSAGE_QUEUE_SIZE));
        const EVENTS_CAPACITY: usize = 4096;
        let events = VecDeque::with_capacity(EVENTS_CAPACITY);

        // create temp mix buffer
        let temp_out = vec![0.0; Self::MAX_MIX_BUFFER_SAMPLES];

        Self {
            playing_sources,
            mixers,
            events,
            effects,
            message_queue,
            channel_count,
            sample_rate,
            temp_out,
        }
    }

    /// Allows controlling the mixer by pushing messages into this event queue.
    /// NB: When adding new sources, ensure they match the mixers sample rate and channel layout
    pub(crate) fn message_queue(&self) -> Arc<ArrayQueue<MixerSourceMessage>> {
        self.message_queue.clone()
    }

    /// remove all entries from self.playing_sources which match the given filter function.
    fn remove_matching_sources<F>(&mut self, match_fn: F)
    where
        F: Fn(&PlayingSource) -> bool,
    {
        self.playing_sources.retain(move |p| !match_fn(p));
    }

    /// remove all entries from self.events which match the given filter function.
    fn remove_matching_events<F>(&mut self, match_fn: F)
    where
        F: Fn(&MixerEvent) -> bool,
    {
        self.events.retain(move |p| !match_fn(p));
    }

    /// remove all entries from self.playing_sources and flush all pending events.
    fn remove_all_playing_sources(&mut self) {
        self.playing_sources.clear();
        self.events.clear();
    }

    /// Process pending mixer messages
    fn process_messages(&mut self, time: &SourceTime) {
        let mut sources_added = false;
        let mut events_added = false;
        while let Some(event) = self.message_queue.pop() {
            match event {
                MixerSourceMessage::AddSource {
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
                    self.playing_sources.push(PlayingSource {
                        is_active: true,
                        playback_id,
                        playback_message_queue,
                        source,
                        start_time: sample_time,
                        stop_time: None,
                    });
                }
                MixerSourceMessage::AddEffect { id, effect } => {
                    self.effects.push((id, effect));
                }
                MixerSourceMessage::AddMixer { id, mixer } => {
                    self.mixers.push((id, mixer));
                }
                MixerSourceMessage::ProcessEffectMessage {
                    effect_id,
                    message,
                    sample_time,
                } => {
                    self.events.push_back(MixerEvent::ProcessEffectMessage {
                        effect_id,
                        message,
                        sample_time,
                    });
                    events_added = true;
                }
                MixerSourceMessage::SetSpeed {
                    playback_id,
                    speed,
                    glide,
                    sample_time,
                } => {
                    self.events.push_back(MixerEvent::SetFileSourceSpeed {
                        playback_id,
                        speed,
                        glide,
                        sample_time,
                    });
                    events_added = true;
                }
                MixerSourceMessage::StopSource {
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
                MixerSourceMessage::RemoveAllPendingSources => {
                    // remove all sources which are not yet playing
                    self.remove_matching_sources(|source| source.start_time > time.pos_in_frames);
                    self.remove_matching_events(|event| event.sample_time() > time.pos_in_frames);
                }
                MixerSourceMessage::RemoveAllSources => {
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

        // Sort events by sample time if any new events were added.
        if events_added {
            bubble_sort_cmp(self.events.make_contiguous(), |a, b| {
                a.sample_time().cmp(&b.sample_time()) as isize
            });
        }
    }

    // Process pending mixer events that are due at the current time
    fn process_events(&mut self, current_time: u64) {
        while self
            .events
            .front()
            .is_some_and(|e| e.sample_time() <= current_time)
        {
            let event = self.events.pop_front().unwrap();
            match event {
                MixerEvent::SetFileSourceSpeed {
                    playback_id,
                    speed,
                    glide,
                    ..
                } => {
                    if let Some(source) = self
                        .playing_sources
                        .iter()
                        .find(|s| s.playback_id == playback_id)
                    {
                        if let PlaybackMessageSender::File(queue) = &source.playback_message_queue {
                            if queue
                                .push(FilePlaybackMessage::SetSpeed(speed, glide))
                                .is_err()
                            {
                                log::warn!("failed to send set speed event");
                            }
                        }
                    }
                }
                MixerEvent::ProcessEffectMessage {
                    effect_id, message, ..
                } => {
                    if let Some((_, effect)) =
                        self.effects.iter_mut().find(|(id, _)| *id == effect_id)
                    {
                        effect.process_message(&**message);
                    } else {
                        log::warn!("Effect with id {effect_id} not found for scheduled message");
                    }
                }
            }
        }
    }

    // Write and mix down all playing sources into the given buffer at the given time.
    fn process_sources(&mut self, output: &mut [f32], time: SourceTime) {
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
        // Process all pending messages
        self.process_messages(time);

        // Return early and avoid touching the buffer if there's nothing to do
        if self.playing_sources.is_empty()
            && self.effects.is_empty()
            && self.mixers.is_empty()
            && self.events.is_empty()
        {
            return 0;
        }

        // Clear entire output first: output should be silent when there are no sources
        clear_buffer(output);

        // Process pending events, sub mixers, sources and effects
        let output_frame_count = output.len() / self.channel_count;
        let mut total_frames_written = 0;

        while total_frames_written < output_frame_count {
            let current_time_in_frames = time.pos_in_frames + total_frames_written as u64;

            // process pending events
            self.process_events(current_time_in_frames);

            // determine how many frames to process until the next event is due
            let frames_to_process = {
                let frames_remaining = output_frame_count - total_frames_written;
                let frames_in_temp_out = self.temp_out.len() / self.channel_count;
                let frames_until_next_event = self.events.front().map_or(usize::MAX, |e| {
                    (e.sample_time() - current_time_in_frames) as usize
                });
                frames_remaining
                    .min(frames_in_temp_out)
                    .min(frames_until_next_event)
            };

            // process next chunk until we reach an event or end of the output
            if frames_to_process > 0 {
                let chunk_time = time.with_added_frames(total_frames_written as u64);
                let chunk_output = &mut output[total_frames_written * self.channel_count
                    ..(total_frames_written + frames_to_process) * self.channel_count];
                let chunk_len = chunk_output.len();

                // apply sub-mixers
                for (_, mixer) in &mut self.mixers {
                    let temp_output = &mut self.temp_out[..chunk_len];
                    let written = mixer.write(temp_output, &chunk_time);
                    // add each sub mixer output to the main output buffer
                    add_buffers(&mut chunk_output[..written], &temp_output[..written]);
                }

                // apply sources
                self.process_sources(chunk_output, chunk_time);

                // apply effects
                for (_, effect) in &mut self.effects {
                    effect.process(chunk_output, &chunk_time);
                }

                total_frames_written += frames_to_process;
            }
        }

        // drop all sources which finished playing in this iteration
        self.remove_matching_sources(|s| !s.is_active);

        // Return output len as we've cleared the entire output before processing
        output.len()
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        // mixer never is exhausted, as we may get new sources added any time
        false
    }
}
