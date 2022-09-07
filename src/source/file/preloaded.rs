use std::{thread, time::Duration};

use crossbeam_channel::{unbounded, Receiver, Sender};

use crate::error::Error;

use super::{
    streamed::StreamedFileSource, FileId, FilePlaybackMsg, FilePlaybackStatusMsg, FileSource,
};
use crate::source::AudioSource;

// -------------------------------------------------------------------------------------------------

/// Preloaded file source
pub struct PreloadedFileSource {
    file_id: FileId,
    file_path: String,
    worker_send: Sender<FilePlaybackMsg>,
    worker_recv: Receiver<FilePlaybackMsg>,
    playback_status_send: Option<Sender<FilePlaybackStatusMsg>>,
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
        file_path: String,
        event_send: Option<Sender<FilePlaybackStatusMsg>>,
    ) -> Result<Self, Error> {
        // create file source
        let mut decoded_file = StreamedFileSource::new(file_path.clone(), None)?;
        let sample_rate = decoded_file.sample_rate();
        let channel_count = decoded_file.channel_count();
        let file_id = decoded_file.file_id();
        let precision = (sample_rate as f64
            * channel_count as f64
            * StreamedFileSource::REPORT_PRECISION.as_secs_f64()) as u64;
        let buffer_capacity = if let Some(total_samples) = decoded_file.total_samples() {
            total_samples as usize
        } else {
            16 * 1024_usize
        };
        // create worker channel
        let (worker_send, worker_recv) = unbounded::<FilePlaybackMsg>();
        // write source into buffer
        let mut temp_buffer: Vec<f32> = vec![0.0; 1024];
        let mut buffer = Vec::with_capacity(buffer_capacity);
        loop {
            let written = decoded_file.write(&mut temp_buffer[..]);
            if written > 0 {
                buffer.append(&mut temp_buffer[..written].to_vec());
            } else if decoded_file.end_of_track() {
                break;
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }
        Ok(Self {
            file_id,
            file_path,
            worker_recv,
            worker_send,
            playback_status_send: event_send,
            buffer,
            buffer_pos: 0_u64,
            channel_count,
            sample_rate,
            report_precision: precision,
            reported_pos: None,
            end_of_track: false,
        })
    }

    fn sender(&self) -> Sender<FilePlaybackMsg> {
        self.worker_send.clone()
    }

    fn file_id(&self) -> FileId {
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
        // consume worked messages
        while let Ok(msg) = self.worker_recv.try_recv() {
            match msg {
                FilePlaybackMsg::Seek(position) => {
                    let buffer_pos = position.as_secs_f64()
                        * self.sample_rate as f64
                        * self.channel_count as f64;
                    self.buffer_pos = (buffer_pos as u64).clamp(0, self.buffer.len() as u64);
                }
                FilePlaybackMsg::Read => (),
                FilePlaybackMsg::Stop => self.buffer_pos = self.buffer.len() as u64,
            }
        }
        // quickly bail out when we finished playing
        if self.end_of_track {
            return 0;
        }
        // write preloaded source at current position
        let pos = self.buffer_pos as usize;
        let remaining = self.buffer.len() - pos;
        let remaining_buffer = &self.buffer[pos..pos + remaining];
        for (o, i) in output.iter_mut().zip(remaining_buffer.iter()) {
            *o = *i;
        }
        // send playback events
        self.buffer_pos += remaining.min(output.len()) as u64;
        if let Some(event_send) = &self.playback_status_send {
            if self.should_report_pos(self.buffer_pos) {
                self.reported_pos = Some(self.buffer_pos);
                if let Err(err) = event_send.try_send(FilePlaybackStatusMsg::Position {
                    file_id: self.file_id,
                    file_path: self.file_path.clone(),
                    position: self.samples_to_duration(self.buffer_pos),
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }
        if self.buffer_pos >= self.buffer.len() as u64 {
            self.end_of_track = true;
            if let Some(event_send) = &self.playback_status_send {
                if let Err(err) = event_send.try_send(FilePlaybackStatusMsg::EndOfFile {
                    file_id: self.file_id,
                    file_path: self.file_path.clone(),
                }) {
                    log::warn!("Failed to send playback event: {}", err)
                }
            }
        }
        remaining.min(output.len())
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
