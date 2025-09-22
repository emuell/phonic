use std::{collections::VecDeque, sync::Arc, time::Duration};

use basedrop::Owned;
use crossbeam_queue::ArrayQueue;

use crate::{
    effect::{Effect, EffectMessage},
    player::{EffectId, PlaybackMessageQueue},
    source::{
        amplified::AmplifiedSourceMessage, file::FilePlaybackMessage, panned::PannedSourceMessage,
        Source, SourceTime,
    },
    utils::{
        buffer::{add_buffers, clear_buffer},
        event::{Event, EventProcessor},
    },
    PlaybackId,
};

// -------------------------------------------------------------------------------------------------

/// Mixer internal struct to keep track of currently playing sources.
pub(crate) struct PlayingSource {
    is_active: bool,
    playback_id: PlaybackId,
    playback_message_queue: PlaybackMessageQueue,
    volume_message_queue: Arc<ArrayQueue<AmplifiedSourceMessage>>,
    panning_message_queue: Arc<ArrayQueue<PannedSourceMessage>>,
    source: Owned<Box<dyn Source>>,
    start_time: u64,
    stop_time: Option<u64>,
}

// -------------------------------------------------------------------------------------------------

/// Mixer internal struct to apply sample time tagged playback events.
pub(crate) enum MixerEvent {
    SeekSource {
        playback_id: PlaybackId,
        position: Duration,
        sample_time: u64,
    },
    SetSourceSpeed {
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>,
        sample_time: u64,
    },
    SetSourceVolume {
        playback_id: PlaybackId,
        volume: f32,
        sample_time: u64,
    },
    SetSourcePanning {
        playback_id: PlaybackId,
        panning: f32,
        sample_time: u64,
    },
    ProcessEffectMessage {
        effect_id: EffectId,
        message: Owned<Box<dyn EffectMessage>>,
        sample_time: u64,
    },
}

impl Event for MixerEvent {
    fn sample_time(&self) -> u64 {
        match self {
            Self::SeekSource { sample_time, .. } => *sample_time,
            Self::SetSourceSpeed { sample_time, .. } => *sample_time,
            Self::SetSourceVolume { sample_time, .. } => *sample_time,
            Self::SetSourcePanning { sample_time, .. } => *sample_time,
            Self::ProcessEffectMessage { sample_time, .. } => *sample_time,
        }
    }
}

// -------------------------------------------------------------------------------------------------

/// Messages send from player to mixer to start or stop playing sources.
pub(crate) enum MixerMessage {
    AddSource {
        playback_id: PlaybackId,
        playback_message_queue: PlaybackMessageQueue,
        volume_message_queue: Arc<ArrayQueue<AmplifiedSourceMessage>>,
        panning_message_queue: Arc<ArrayQueue<PannedSourceMessage>>,
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
    SetSourceSpeed {
        playback_id: PlaybackId,
        speed: f64,
        glide: Option<f32>, // semitones per second
        sample_time: u64,
    },
    SetSourceVolume {
        playback_id: PlaybackId,
        volume: f32,
        sample_time: u64,
    },
    SetSourcePanning {
        playback_id: PlaybackId,
        panning: f32,
        sample_time: u64,
    },
    SeekSource {
        playback_id: PlaybackId,
        position: Duration,
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
    playing_sources: VecDeque<PlayingSource>,
    mixers: Vec<(EffectId, Box<MixedSource>)>,
    effects: Vec<(EffectId, Box<dyn Effect>)>,
    message_queue: Arc<ArrayQueue<MixerMessage>>,
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
        let playing_sources = VecDeque::with_capacity(PLAYING_EVENTS_CAPACITY);
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
    pub(crate) fn message_queue(&self) -> Arc<ArrayQueue<MixerMessage>> {
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
        while let Some(event) = self.message_queue.pop() {
            match event {
                MixerMessage::AddSource {
                    playback_id,
                    playback_message_queue,
                    volume_message_queue,
                    panning_message_queue,
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
                    // sort playing_sources by start time
                    let playing_sources = &mut self.playing_sources;
                    let insert_pos = playing_sources
                        .make_contiguous()
                        .partition_point(|e| e.start_time < sample_time);
                    playing_sources.insert(
                        insert_pos,
                        PlayingSource {
                            is_active: true,
                            playback_id,
                            playback_message_queue,
                            volume_message_queue,
                            panning_message_queue,
                            source,
                            start_time: sample_time,
                            stop_time: None,
                        },
                    );
                }
                MixerMessage::AddEffect { id, effect } => {
                    self.effects.push((id, effect));
                }
                MixerMessage::AddMixer { id, mixer } => {
                    self.mixers.push((id, mixer));
                }
                MixerMessage::ProcessEffectMessage {
                    effect_id,
                    message,
                    sample_time,
                } => {
                    self.insert_event(MixerEvent::ProcessEffectMessage {
                        effect_id,
                        message,
                        sample_time,
                    });
                }
                MixerMessage::SetSourceSpeed {
                    playback_id,
                    speed,
                    glide,
                    sample_time,
                } => {
                    self.insert_event(MixerEvent::SetSourceSpeed {
                        playback_id,
                        speed,
                        glide,
                        sample_time,
                    });
                }
                MixerMessage::SetSourceVolume {
                    playback_id,
                    volume,
                    sample_time,
                } => {
                    self.insert_event(MixerEvent::SetSourceVolume {
                        playback_id,
                        volume,
                        sample_time,
                    });
                }
                MixerMessage::SetSourcePanning {
                    playback_id,
                    panning,
                    sample_time,
                } => {
                    self.insert_event(MixerEvent::SetSourcePanning {
                        playback_id,
                        panning,
                        sample_time,
                    });
                }
                MixerMessage::SeekSource {
                    playback_id,
                    position,
                    sample_time,
                } => {
                    self.insert_event(MixerEvent::SeekSource {
                        playback_id,
                        position,
                        sample_time,
                    });
                }
                MixerMessage::StopSource {
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
                MixerMessage::RemoveAllPendingSources => {
                    // remove all sources which are not yet playing
                    self.remove_matching_sources(|source| source.start_time > time.pos_in_frames);
                    self.remove_matching_events(|event| event.sample_time() > time.pos_in_frames);
                }
                MixerMessage::RemoveAllSources => {
                    self.remove_all_playing_sources();
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
                let frames_until_next_event = self.time_until_next_event(current_time_in_frames);
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

impl EventProcessor for MixedSource {
    type Event = MixerEvent;

    fn events(&self) -> &VecDeque<Self::Event> {
        &self.events
    }
    fn events_mut(&mut self) -> &mut VecDeque<Self::Event> {
        &mut self.events
    }

    fn process_event(&mut self, event: Self::Event) {
        match event {
            MixerEvent::SeekSource {
                playback_id,
                position,
                sample_time: _,
            } => {
                if let Some(source) = self
                    .playing_sources
                    .iter_mut()
                    .find(|s| s.playback_id == playback_id)
                {
                    if let PlaybackMessageQueue::File(queue) = &source.playback_message_queue {
                        if let Err(msg) = queue.push(FilePlaybackMessage::Seek(position)) {
                            log::warn!("Failed to send seek command to file. Force pushing it...");
                            let _ = queue.force_push(msg);
                        }
                    } else {
                        log::warn!("Trying to seek a synth source, which is not supported");
                    }
                }
            }
            MixerEvent::SetSourceSpeed {
                playback_id,
                speed,
                glide,
                sample_time: _,
            } => {
                if let Some(source) = self
                    .playing_sources
                    .iter()
                    .find(|s| s.playback_id == playback_id)
                {
                    if let PlaybackMessageQueue::File(queue) = &source.playback_message_queue {
                        if let Err(msg) = queue.push(FilePlaybackMessage::SetSpeed(speed, glide)) {
                            log::warn!("Failed to send set speed event. Force pushing it...");
                            let _ = queue.force_push(msg);
                        }
                    }
                }
            }
            MixerEvent::SetSourceVolume {
                playback_id,
                volume,
                sample_time: _,
            } => {
                if let Some(source) = self
                    .playing_sources
                    .iter()
                    .find(|s| s.playback_id == playback_id)
                {
                    if let Err(msg) = source
                        .volume_message_queue
                        .push(AmplifiedSourceMessage::SetVolume(volume))
                    {
                        log::warn!("Failed to send set volume event. Force pushing it...");
                        let _ = source.volume_message_queue.force_push(msg);
                    }
                }
            }
            MixerEvent::SetSourcePanning {
                playback_id,
                panning,
                sample_time: _,
            } => {
                if let Some(source) = self
                    .playing_sources
                    .iter()
                    .find(|s| s.playback_id == playback_id)
                {
                    if let Err(msg) = source
                        .panning_message_queue
                        .push(PannedSourceMessage::SetPanning(panning))
                    {
                        log::warn!("Failed to send set panning event. Force pushing it...");
                        let _ = source.panning_message_queue.force_push(msg);
                    }
                }
            }
            MixerEvent::ProcessEffectMessage {
                effect_id,
                message,
                sample_time: _,
            } => {
                if let Some((_, effect)) = self.effects.iter_mut().find(|(id, _)| *id == effect_id)
                {
                    effect.process_message(&**message);
                } else {
                    log::warn!("Effect with id {effect_id} not found for scheduled message");
                }
            }
        }
    }
}
