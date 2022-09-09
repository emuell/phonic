use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};
use symphonia::core::audio::SampleBuffer;

use super::{streamed::StreamedFileSource, FilePlaybackMessage, FileSource};
use crate::{
    error::Error,
    source::{
        file::{PlaybackId, PlaybackStatusEvent},
        AudioSource,
    },
    utils::{decoder::AudioDecoder, unique_usize_id},
};

// -------------------------------------------------------------------------------------------------

/// Preloaded file source
pub struct PreloadedFileSource {
    file_id: PlaybackId,
    file_path: String,
    volume: f32,
    repeat: usize,
    playback_message_send: Sender<FilePlaybackMessage>,
    playback_message_receive: Receiver<FilePlaybackMessage>,
    playback_status_send: Option<Sender<PlaybackStatusEvent>>,
    buffer: Vec<f32>,
    buffer_pos: u64,
    channel_count: usize,
    sample_rate: u32,
    report_precision: u64,
    reported_pos: Option<u64>,
    end_of_track: bool,
}

impl PreloadedFileSource {
    fn should_report_pos(&self, pos: u64) -> bool {
        if let Some(reported) = self.reported_pos {
            reported > pos || pos - reported >= self.report_precision
        } else {
            true
        }
    }

    fn samples_to_duration(&self, samples: u64) -> Duration {
        let frames = samples / self.channel_count as u64;
        let seconds = frames as f64 / self.sample_rate as f64;
        Duration::from_millis((seconds * 1000.0) as u64)
    }
}

impl FileSource for PreloadedFileSource {
    fn new(
        file_path: &str,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        volume: f32,
        repeat: usize,
    ) -> Result<Self, Error> {
        // create decoder and get signal specs
        let mut audio_decoder = AudioDecoder::new(file_path.to_string())?;
        let sample_rate = audio_decoder.signal_spec().rate;
        let channel_count = audio_decoder.signal_spec().channels.count();

        // create a channel for playback messages
        let (playback_message_send, playback_message_receive) = unbounded::<FilePlaybackMessage>();

        // decode the entire file into our buffer
        let buffer_capacity = if let Some(total_frames) = audio_decoder.codec_params().n_frames {
            // Note: this is a hint only!
            total_frames as usize * channel_count
        } else {
            16 * 1024_usize
        };
        let mut buffer = Vec::with_capacity(buffer_capacity);

        let mut temp_sample_buffer = SampleBuffer::<f32>::new(
            audio_decoder
                .codec_params()
                .max_frames_per_packet
                .unwrap_or(16 * 1024 * channel_count as u64),
            audio_decoder.signal_spec(),
        );
        while audio_decoder.read_packet(&mut temp_sample_buffer).is_some() {
            buffer.append(&mut temp_sample_buffer.samples().to_vec());
        }
        // TODO: should pass a proper error here
        if buffer.is_empty() {
            return Err(Error::AudioDecodingError(Box::new(
                symphonia::core::errors::Error::DecodeError("failed to decode file"),
            )));
        }

        let report_precision = (sample_rate as f64
            * channel_count as f64
            * StreamedFileSource::REPORT_PRECISION.as_secs_f64())
            as u64;

        Ok(Self {
            file_id: unique_usize_id(),
            file_path: file_path.to_string(),
            volume,
            repeat,
            playback_message_receive,
            playback_message_send,
            playback_status_send,
            buffer,
            buffer_pos: 0_u64,
            channel_count,
            sample_rate,
            report_precision,
            reported_pos: None,
            end_of_track: false,
        })
    }

    fn playback_message_sender(&self) -> Sender<FilePlaybackMessage> {
        self.playback_message_send.clone()
    }

    fn playback_id(&self) -> PlaybackId {
        self.file_id
    }

    fn total_frames(&self) -> Option<u64> {
        Some(self.buffer.len() as u64 / self.channel_count() as u64)
    }

    fn current_frame_position(&self) -> u64 {
        self.buffer_pos / self.channel_count() as u64
    }

    fn end_of_track(&self) -> bool {
        self.end_of_track
    }
}

impl AudioSource for PreloadedFileSource {
    fn write(&mut self, output: &mut [f32]) -> usize {
        // consume playback messages
        let mut keep_running = true;
        while let Ok(msg) = self.playback_message_receive.try_recv() {
            match msg {
                FilePlaybackMessage::Seek(position) => {
                    let buffer_pos = position.as_secs_f64()
                        * self.sample_rate as f64
                        * self.channel_count as f64;
                    self.buffer_pos = (buffer_pos as u64).clamp(0, self.buffer.len() as u64);
                }
                FilePlaybackMessage::Read => (),
                FilePlaybackMessage::Stop => keep_running = false,
            }
        }

        // quickly bail out when we finished playing
        if self.end_of_track {
            return 0;
        }

        // write preloaded source at current position and apply volume
        let mut total_written = 0_usize;
        while total_written < output.len() {
            let pos = self.buffer_pos as usize;
            let remaining = self.buffer.len() - pos;
            let remaining_buffer = &self.buffer[pos..pos + remaining];
            let target = &mut output[total_written..];
            for (o, i) in target.iter_mut().zip(remaining_buffer.iter()) {
                *o = *i * self.volume;
            }
            let written = remaining.min(target.len());
            self.buffer_pos += written as u64;
            total_written += written;
            // loop or stop when reaching end of file
            if self.buffer_pos >= self.buffer.len() as u64 {
                if self.repeat > 0 {
                    if self.repeat != usize::MAX {
                        self.repeat -= 1;
                    }
                    self.buffer_pos = 0;
                    self.reported_pos = None; // force reporting a new pos
                } else {
                    break;
                }
            }
        }

        // send playback events
        if let Some(event_send) = &self.playback_status_send {
            if self.should_report_pos(self.buffer_pos) {
                self.reported_pos = Some(self.buffer_pos);
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Position {
                    id: self.file_id,
                    path: self.file_path.clone(),
                    position: self.samples_to_duration(self.buffer_pos),
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }
        if self.buffer_pos >= self.buffer.len() as u64 || !keep_running {
            self.end_of_track = true;
            if let Some(event_send) = &self.playback_status_send {
                if let Err(err) = event_send.try_send(PlaybackStatusEvent::Stopped {
                    id: self.file_id,
                    path: self.file_path.clone(),
                    exhausted: self.buffer_pos >= self.buffer.len() as u64,
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }

        total_written as usize
    }

    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.end_of_track
    }
}
