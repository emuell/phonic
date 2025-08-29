use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;

use crate::{
    error::Error,
    player::{PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
    source::{
        file::{FilePlaybackMessage, FilePlaybackOptions},
        resampled::ResamplingQuality,
    },
    utils::{
        buffer::TempBuffer,
        fader::VolumeFader,
        resampler::{
            cubic::CubicResampler, rubato::RubatoResampler, AudioResampler, ResamplingSpecs,
        },
        unique_usize_id,
    },
};

// -------------------------------------------------------------------------------------------------

// Common, shared file source implementation helper
pub struct FileSourceImpl {
    pub file_id: PlaybackId,
    pub file_path: Arc<String>,
    pub options: FilePlaybackOptions,
    pub volume_fader: VolumeFader,
    pub resampler: Box<dyn AudioResampler>,
    pub resampler_input_buffer: TempBuffer,
    pub fade_out_duration: Option<Duration>,
    pub output_sample_rate: u32,
    pub output_channel_count: usize,
    pub playback_message_queue: Arc<ArrayQueue<FilePlaybackMessage>>,
    pub playback_status_send: Option<Sender<PlaybackStatusEvent>>,
    pub playback_status_context: Option<PlaybackStatusContext>,
    pub playback_pos_report_instant: Instant,
    pub playback_pos_emit_rate: Option<Duration>,
    pub playback_finished: bool,
    pub samples_to_next_speed_update: usize,
    pub speed_glide_rate: f32,
    pub current_speed: f64,
    pub target_speed: f64,
}

impl FileSourceImpl {
    /// Speed changes should be applied in sample blocks of this size in all FileSource impls.
    pub(crate) const SPEED_UPDATE_CHUNK_SIZE: usize = 64;

    pub fn new(
        file_path: &str,
        options: FilePlaybackOptions,
        input_sample_rate: u32,
        input_channel_count: usize,
        output_sample_rate: u32,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
    ) -> Result<Self, Error> {
        // create event queue for the player
        let playback_message_queue = Arc::new(ArrayQueue::new(128));

        // create new volume fader
        let mut volume_fader = VolumeFader::new(input_channel_count, output_sample_rate);
        if let Some(duration) = options.fade_in_duration {
            if !duration.is_zero() {
                volume_fader.start_fade_in(duration);
            }
        }

        // create resampler
        let resampler_specs = ResamplingSpecs::new(
            input_sample_rate,
            (output_sample_rate as f64 / options.speed) as u32,
            input_channel_count,
        );
        let resampler: Box<dyn AudioResampler> = match options.resampling_quality {
            ResamplingQuality::HighQuality => Box::new(RubatoResampler::new(resampler_specs)?),
            ResamplingQuality::Default => Box::new(CubicResampler::new(resampler_specs)?),
        };
        const DEFAULT_CHUNK_SIZE: usize = 256;
        let resample_input_buffer_size = resampler
            .max_input_buffer_size()
            .unwrap_or(DEFAULT_CHUNK_SIZE);
        let resampler_input_buffer = TempBuffer::new(resample_input_buffer_size);

        // create new unique file id
        let file_id = unique_usize_id();
        let file_path = Arc::new(file_path.to_owned());

        // create empty context
        let playback_status_context = None;

        // copy and initialize options which are applied while playback
        let playback_pos_report_instant = Instant::now();
        let playback_pos_emit_rate = options.playback_pos_emit_rate;
        let playback_finished = false;

        let fade_out_duration = options.fade_out_duration;

        let output_channel_count = input_channel_count;

        let samples_to_next_speed_update = 0;
        let current_speed = options.speed;
        let target_speed = options.speed;
        let speed_glide_rate = 0.0;

        Ok(Self {
            file_id,
            file_path,
            options,
            volume_fader,
            resampler,
            resampler_input_buffer,
            fade_out_duration,
            output_sample_rate,
            output_channel_count,
            playback_message_queue,
            playback_status_send,
            playback_status_context,
            playback_pos_report_instant,
            playback_pos_emit_rate,
            playback_finished,
            samples_to_next_speed_update,
            speed_glide_rate,
            current_speed,
            target_speed,
        })
    }

    pub fn update_speed(&mut self, input_sample_rate: u32) {
        // ramp current speed to target
        let speed_diff = self.target_speed - self.current_speed;
        if self.speed_glide_rate > 0.0 && speed_diff.abs() > 0.0001 {
            let semitone_diff = (12.0 * (self.target_speed / self.current_speed).log2()).abs();
            let duration_secs = semitone_diff as f32 / self.speed_glide_rate;
            if duration_secs > 0.0 {
                let duration_frames = duration_secs * self.output_sample_rate as f32;
                let speed_step_per_frame =
                    (self.target_speed - self.current_speed) / duration_frames as f64;
                let speed_change_this_call =
                    speed_step_per_frame * Self::SPEED_UPDATE_CHUNK_SIZE as f64;
                if (self.target_speed - self.current_speed).abs() < speed_change_this_call.abs() {
                    self.current_speed = self.target_speed;
                } else {
                    self.current_speed += speed_change_this_call;
                }
            } else {
                self.current_speed = self.target_speed;
            }
        } else {
            self.current_speed = self.target_speed;
        }
        // update resampler with new speed
        let new_output_rate = (self.output_sample_rate as f64 / self.current_speed) as u32;
        self.resampler
            .update(input_sample_rate, new_output_rate)
            .expect("failed to update resampler specs");
    }

    pub fn should_report_pos(&self) -> bool {
        if let Some(report_duration) = self.playback_pos_emit_rate {
            self.playback_pos_report_instant.elapsed() >= report_duration
        } else {
            false
        }
    }

    pub fn samples_to_duration(&self, samples: u64, channel_count: usize) -> Duration {
        let frames = samples / channel_count as u64;
        let seconds = frames as f64 / self.output_sample_rate as f64;
        Duration::from_secs_f64(seconds)
    }

    pub fn send_playback_position_status(&mut self, position: Duration) {
        if let Some(event_send) = &self.playback_status_send {
            if self.should_report_pos() {
                self.playback_pos_report_instant = Instant::now();
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Position {
                    id: self.file_id,
                    context: self.playback_status_context.clone(),
                    path: self.file_path.clone(),
                    position,
                }) {
                    log::warn!("failed to send playback event: {err}")
                }
            }
        }
    }

    pub fn send_playback_stopped_status(&mut self, is_exhausted: bool) {
        if let Some(event_send) = &self.playback_status_send {
            if let Err(err) = event_send.try_send(PlaybackStatusEvent::Stopped {
                id: self.file_id,
                context: self.playback_status_context.clone(),
                path: self.file_path.clone(),
                exhausted: is_exhausted,
            }) {
                log::warn!("failed to send playback event: {err}")
            }
        }
    }
}
