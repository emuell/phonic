use crate::{
    source::{converted::ConvertedSource, AudioSource},
    utils::resampler::ResamplingQuality,
};

use crossbeam_channel::{unbounded, Receiver, Sender};

// -------------------------------------------------------------------------------------------------

struct PlayingSource {
    is_active: bool,
    source: Box<dyn AudioSource>,
    sample_time: u64,
}

// -------------------------------------------------------------------------------------------------

pub enum MixedSourceMsg {
    AddSource {
        source: Box<dyn AudioSource>,
        sample_time: u64,
    },
    RemoveAllSources,
    RemoveAllPendingSources,
}

// -------------------------------------------------------------------------------------------------

/// A source which converts and mixes other sources together.
pub struct MixedSource {
    playing_sources: Vec<PlayingSource>,
    playback_pos: u64,
    event_send: Sender<MixedSourceMsg>,
    event_recv: Receiver<MixedSourceMsg>,
    channel_count: usize,
    sample_rate: u32,
    temp_out: Vec<f32>,
}

impl MixedSource {
    /// Create a new mixer source with the given signal specs.
    /// Param `sample_time` is the intial sample frame time that we start to run with.
    /// This usually will be the audio outputs playback pos.
    pub fn new(channel_count: usize, sample_rate: u32, sample_time: u64) -> Self {
        let (event_send, event_recv) = unbounded::<MixedSourceMsg>();
        // temp mix buffer size
        const BUFFER_SIZE: usize = 8 * 1024;
        // avoid allocs in real-time threads
        const PLAYING_EVENTS_CAPACITY: usize = 1024;
        Self {
            playing_sources: Vec::with_capacity(PLAYING_EVENTS_CAPACITY),
            playback_pos: sample_time,
            event_recv,
            event_send,
            channel_count,
            sample_rate,
            temp_out: vec![0.0; BUFFER_SIZE],
        }
    }

    /// Add a source to the mix
    pub fn add(&mut self, source: impl AudioSource, quality: ResamplingQuality) {
        let sample_time = 0;
        self.add_at_sample_time(source, sample_time, quality);
    }

    /// Add a source to the mix scheduling it to play at the given absolute sample time.
    pub fn add_at_sample_time(
        &mut self,
        source: impl AudioSource,
        sample_time: u64,
        quality: ResamplingQuality,
    ) {
        let converted = Box::new(ConvertedSource::new(
            source,
            self.channel_count,
            self.sample_rate,
            quality,
        ));
        if let Err(err) = self.event_send.send(MixedSourceMsg::AddSource {
            source: converted,
            sample_time,
        }) {
            log::error!("Failed to add mixer source: {}", { err });
        }
    }

    /// Allows controlling the mixer via a message channel.
    /// NB: When adding new sources, ensure they match the mixers sample rate and channel layout
    pub(crate) fn event_sender(&self) -> crossbeam_channel::Sender<MixedSourceMsg> {
        self.event_send.clone()
    }
}

impl AudioSource for MixedSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        // process events
        let mut got_new_sources = false;
        while let Ok(event) = self.event_recv.try_recv() {
            match event {
                MixedSourceMsg::AddSource {
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
                    got_new_sources = true;
                    self.playing_sources.push(PlayingSource {
                        is_active: true,
                        source,
                        sample_time,
                    });
                }
                MixedSourceMsg::RemoveAllPendingSources => {
                    // remove all sources which are not yet playing
                    let playback_pos = self.playback_pos;
                    self.playing_sources
                        .retain(|source| source.sample_time <= playback_pos);
                }
                MixedSourceMsg::RemoveAllSources => {
                    // remove all sources
                    self.playing_sources.clear();
                }
            }
        }
        // keep sources sorted by sample time: this makes batch processing easier
        if got_new_sources {
            self.playing_sources
                .sort_by(|a, b| a.sample_time.cmp(&b.sample_time));
        }

        // return empty handed when we have no sources
        let output_frame_count = output.len() / self.channel_count;
        if self.playing_sources.is_empty() {
            // but move our playback sample counter
            self.playback_pos += output_frame_count as u64;
            return 0;
        }
        // clear output as we're only adding below
        for o in output.iter_mut() {
            *o = 0_f32;
        }
        // run and add all playing sources
        let mut max_written = 0usize;
        'all_sources: for playing_source in self.playing_sources.iter_mut() {
            let source = &mut playing_source.source;
            let mut total_written: usize = 0;
            // check source's sample start time
            if playing_source.sample_time > self.playback_pos {
                let frames_until_source_starts =
                    (playing_source.sample_time - self.playback_pos) as usize * self.channel_count;
                if frames_until_source_starts > 0 {
                    if frames_until_source_starts >= output_frame_count {
                        // playing_sources are sorted by sample time: all following sources will run
                        // after this source, and thus also can also be skipped...
                        break 'all_sources;
                    }
                    // offset to the sample start
                    total_written += frames_until_source_starts;
                }
            }
            // run and mix down the source
            'source: while total_written < output.len() {
                // run source on temp_out until we've filled up the whole final output
                let remaining = output.len() - total_written;
                let to_write = remaining.min(self.temp_out.len());
                let written = source.write(&mut self.temp_out[..to_write]);
                if source.is_exhausted() {
                    // source no longer is playing: mark it as inactive
                    playing_source.is_active = false;
                    break 'source;
                }
                // add output of the source to the final output
                let remaining_out = &mut output[total_written..];
                let written_out = &self.temp_out[..written];
                for (o, i) in remaining_out.iter_mut().zip(written_out) {
                    *o += *i;
                }
                total_written += written;
            }
            max_written = max_written.max(total_written);
        }
        // drain inactive sources
        self.playing_sources.retain(|s| s.is_active);
        // move our playback sample counter
        self.playback_pos += output_frame_count as u64;
        // return modified output len: we've cleared the entire output
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
