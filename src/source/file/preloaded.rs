use std::{
    ops::Range,
    path::Path,
    sync::{mpsc::SyncSender, Arc},
    time::Duration,
};

use crossbeam_queue::ArrayQueue;

use super::{common::FileSourceImpl, FilePlaybackMessage, FilePlaybackOptions, FileSource};

use crate::{
    error::Error,
    source::{
        file::{AudioFileBuffer, PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
        Source, SourceTime,
    },
    utils::{buffer::clear_buffer, fader::FaderState},
};

// -------------------------------------------------------------------------------------------------

/// A buffered, clonable [`FileSource`], which decodes the entire file into a buffer before its
/// played back.
///
/// Buffers of preloaded file sources are shared (wrapped in an Arc), so cloning a source is
/// very cheap as this only copies a buffer reference and not the buffer itself. This way a file
/// can be pre-loaded once and can then be cloned and reused as often as necessary.
pub struct PreloadedFileSource {
    file_buffer: Arc<AudioFileBuffer>,
    file_source: FileSourceImpl,
    playback_repeat: usize,
    playback_repeat_count: usize,
    playback_pos: usize,
    playback_pos_eof: bool,
    loop_range_override: Option<Range<u64>>,
}

impl PreloadedFileSource {
    /// Create a new preloaded file source with the given audio file path.
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // Memorize file path for progress
        let file_path = path.as_ref().to_string_lossy().to_string();
        Self::from_shared_buffer(
            Arc::new(AudioFileBuffer::from_file(path)?),
            &file_path,
            options,
            output_sample_rate,
        )
    }

    /// Create a new preloaded file source with the given raw **encoded** file stream buffer.
    pub fn from_file_buffer(
        file_buffer: Vec<u8>,
        file_path: &str,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        Self::from_shared_buffer(
            Arc::new(AudioFileBuffer::from_file_buffer(file_buffer)?),
            file_path,
            options,
            output_sample_rate,
        )
    }

    /// Create a new preloaded file source from the given shared, **decoded** file buffer.
    pub fn from_shared_buffer(
        file_buffer: Arc<AudioFileBuffer>,
        file_path: &str,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // validate options
        options.validate()?;

        // create common data
        let file_source = FileSourceImpl::new(
            file_path,
            options,
            file_buffer.sample_rate(),
            file_buffer.channel_count(),
            output_sample_rate,
        )?;

        let playback_repeat = options
            .repeat
            .unwrap_or(if file_buffer.loop_range().is_some() {
                usize::MAX
            } else {
                0
            });
        let playback_repeat_count = playback_repeat;
        let playback_pos = 0;
        let playback_pos_eof = false;

        let loop_range_override = options.loop_range.map(|(start, end)| {
            let frame_count = file_buffer.frame_count() as u64;
            start.min(frame_count.saturating_sub(1))..end.min(frame_count)
        });

        Ok(Self {
            file_buffer,
            file_source,
            playback_repeat_count,
            playback_repeat,
            playback_pos,
            playback_pos_eof,
            loop_range_override,
        })
    }

    /// Create a copy of this preloaded source with the given playback options.
    pub fn clone(
        &self,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        let file_buffer = Arc::clone(&self.file_buffer);
        Self::from_shared_buffer(
            file_buffer,
            &self.file_source.file_path,
            options,
            output_sample_rate,
        )
    }

    /// Access to the shared file buffer.
    pub fn file_buffer(&self) -> Arc<AudioFileBuffer> {
        Arc::clone(&self.file_buffer)
    }

    /// Set a new playback position for this source.
    pub fn seek(&mut self, position: Duration) {
        if !self.is_exhausted() {
            let buffer_pos = position.as_secs_f64()
                * self.file_buffer.sample_rate() as f64
                * self.file_buffer.channel_count() as f64;
            self.playback_pos = (buffer_pos as usize).clamp(0, self.file_buffer.buffer().len());
            self.file_source.resampler.reset();
        }
    }

    /// Returns the active loop range in sample frames: the override if set, else the file's
    /// embedded loop range. Returns `None` when neither is set.
    pub fn loop_range(&self) -> Option<Range<u64>> {
        self.loop_range_override.clone().or_else(|| {
            self.file_buffer
                .loop_range()
                .map(|r| r.start as u64..r.end as u64)
        })
    }

    /// Override the file's embedded loop range with a custom one.
    /// Pass `None` to revert to the file's embedded loop range.
    pub fn set_loop_range(&mut self, range: Option<Range<u64>>) {
        let frame_count = self.file_buffer.frame_count() as u64;
        assert!(
            range.is_none()
                || range
                    .as_ref()
                    .is_some_and(|r| r.start < frame_count && r.end <= frame_count),
            "Invalid loop range: {:?} not in range {:?}",
            range,
            0..frame_count
        );
        self.loop_range_override = range;
    }

    /// Override the file's playback option repeat settings with the given ones.
    /// Set to 0 to disable looping, usize::MAX to repeat forever.
    pub fn set_repeat(&mut self, repeat_count: usize) {
        self.playback_repeat = repeat_count;
        self.playback_repeat_count = self.playback_repeat;
    }

    /// Set the playback speed (pitch) for this source.
    pub fn set_speed(&mut self, speed: f64, glide: Option<f32>) {
        if !self.is_exhausted() {
            self.file_source.samples_to_next_speed_update = 0;
            self.file_source.target_speed = speed;
            self.file_source.speed_glide_rate = glide.unwrap_or(0.0);
            if self.file_source.speed_glide_rate == 0.0 {
                self.file_source.current_speed = speed;
                self.file_source
                    .update_speed(self.file_buffer.sample_rate());
            }
        }
    }

    /// Stop the file source, starting to fade-out, when a fadeout is set, stop immediately.
    pub fn stop(&mut self) {
        if !self.is_exhausted() {
            match self.file_source.fade_out_duration {
                Some(duration) if !duration.is_zero() => {
                    self.file_source.volume_fader.start_fade_out(duration);
                }
                _ => {
                    self.file_source
                        .send_playback_stopped_status(self.playback_pos_eof);
                    self.file_source.playback_finished = true;
                }
            }
        }
    }

    /// Reset the file to start playback from the beginning.
    pub fn reset(&mut self) {
        // Send stopped status
        if !self.is_exhausted() {
            self.kill();
        }
        // Reset positions and playback status
        self.playback_pos = 0;
        self.playback_repeat_count = self.playback_repeat;
        self.playback_pos_eof = false;
        self.file_source.playback_started = false;
        self.file_source.playback_finished = false;

        // Reset resampler state
        self.file_source.resampler.reset();
        self.file_source.resampler_input_buffer.clear_range();

        // Reset volume fader
        self.file_source.volume_fader.reset();
    }

    /// Abruptly stop the source without applying fade-outs
    pub fn kill(&mut self) {
        if !self.is_exhausted() {
            self.file_source
                .send_playback_stopped_status(self.playback_pos_eof);
            self.file_source.playback_finished = true;
        }
    }

    /// access to the file source impl
    #[allow(unused)]
    pub(crate) fn file_source_impl(&self) -> &FileSourceImpl {
        &self.file_source
    }
    /// Mut access to the file source impl
    pub(crate) fn file_source_impl_mut(&mut self) -> &mut FileSourceImpl {
        &mut self.file_source
    }

    fn process_messages(&mut self) {
        while let Some(msg) = self.file_source.playback_message_queue.pop() {
            match msg {
                FilePlaybackMessage::Seek(position) => {
                    self.seek(position);
                }
                FilePlaybackMessage::SetSpeed(speed, glide) => {
                    self.set_speed(speed, glide);
                }
                FilePlaybackMessage::Stop => {
                    self.stop();
                }
                FilePlaybackMessage::Kill => {
                    self.kill();
                }
            }
        }
    }

    fn write_buffer(&mut self, output: &mut [f32]) -> usize {
        let mut written = 0;

        let loop_range = if self.playback_repeat > 0 {
            let channel_count = self.file_buffer.channel_count();
            self.loop_range()
                .map(|r| r.start as usize * channel_count..r.end as usize * channel_count)
                .unwrap_or(0..self.file_buffer.buffer().len())
        } else {
            0..self.file_buffer.buffer().len()
        };

        let resampler = &mut self.file_source.resampler;
        let resampler_input_buffer = &mut self.file_source.resampler_input_buffer;

        let required_input_len = resampler.required_input_buffer_size().unwrap_or(0);

        while written < output.len() {
            // write from resampled buffer into output and apply volume
            let remaining_input_len = loop_range.end.saturating_sub(self.playback_pos);
            let remaining_input_buffer = &self.file_buffer.buffer()
                [self.playback_pos..self.playback_pos + remaining_input_len];
            let remaining_output = &mut output[written..];
            let (input_consumed, output_written) = {
                // pad input with zeros if resampler has input size constrains.
                // should only happen in the last process call at the EOF
                if remaining_input_buffer.len() < required_input_len {
                    resampler_input_buffer.reset_range();
                    resampler_input_buffer.copy_from(remaining_input_buffer);
                    clear_buffer(&mut resampler_input_buffer.get_mut()[remaining_input_len..]);
                    let (_, output_written) = resampler
                        .process(resampler_input_buffer.get(), remaining_output)
                        .expect("PreloadedFile resampling failed");
                    (remaining_input_len, output_written)
                } else {
                    resampler
                        .process(remaining_input_buffer, remaining_output)
                        .expect("PreloadedFile resampling failed")
                }
            };

            // move buffer read pos
            self.playback_pos += input_consumed;
            written += output_written;

            // loop or stop when reaching end of file or end of loop
            if self.playback_pos >= loop_range.end {
                if self.playback_repeat_count > 0 {
                    if self.playback_repeat_count != usize::MAX {
                        self.playback_repeat_count -= 1;
                    }
                    self.playback_pos = loop_range.start;
                } else {
                    self.playback_pos_eof = true;
                }
            }
            if self.playback_pos_eof && output_written == 0 {
                // got no more output from file or resampler
                break;
            }
        }
        written
    }
}

impl FileSource for PreloadedFileSource {
    fn file_name(&self) -> String {
        self.file_source.file_path.to_string()
    }

    fn playback_id(&self) -> PlaybackId {
        self.file_source.file_id
    }

    fn playback_options(&self) -> &FilePlaybackOptions {
        &self.file_source.options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<FilePlaybackMessage>> {
        Arc::clone(&self.file_source.playback_message_queue)
    }

    fn playback_status_sender(&self) -> Option<SyncSender<PlaybackStatusEvent>> {
        self.file_source.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.file_source.playback_status_send = sender;
    }

    fn playback_status_context(&self) -> Option<PlaybackStatusContext> {
        self.file_source.playback_status_context.clone()
    }
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>) {
        self.file_source.playback_status_context = context;
    }

    fn total_frames(&self) -> Option<u64> {
        Some(self.file_buffer.buffer().len() as u64 / self.file_buffer.channel_count() as u64)
    }

    fn current_frame_position(&self) -> u64 {
        self.playback_pos as u64 / self.file_source.output_channel_count as u64
    }

    fn end_of_track(&self) -> bool {
        self.file_source.playback_finished
    }
}

impl Source for PreloadedFileSource {
    fn channel_count(&self) -> usize {
        self.file_source.output_channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.file_source.output_sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.file_source.playback_finished
    }

    fn weight(&self) -> usize {
        1
    }

    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // consume playback messages
        self.process_messages();

        // quickly bail out when we've finished playing
        if self.file_source.playback_finished {
            return 0;
        }

        // send Position start event, if needed
        if self.file_source.playback_started {
            self.file_source.playback_started = false;
            let is_start_event = true;
            self.file_source.send_playback_position_status(
                time,
                is_start_event,
                self.playback_pos as u64,
                self.file_buffer.channel_count(),
                self.file_buffer.sample_rate(),
            );
        }

        let mut total_written = 0_usize;
        if self.file_source.current_speed != self.file_source.target_speed {
            // update pitch slide in blocks of SPEED_UPDATE_CHUNK_SIZE
            while total_written < output.len() {
                if self.file_source.samples_to_next_speed_update == 0 {
                    if self.file_source.current_speed != self.file_source.target_speed {
                        self.file_source
                            .update_speed(self.file_buffer.sample_rate());
                    }
                    self.file_source.samples_to_next_speed_update =
                        FileSourceImpl::SPEED_UPDATE_CHUNK_SIZE
                            * self.file_source.output_channel_count;
                }
                let chunk_length = (output.len() - total_written)
                    .min(self.file_source.samples_to_next_speed_update);
                let output_chunk = &mut output[total_written..total_written + chunk_length];
                let written = self.write_buffer(output_chunk);

                self.file_source.samples_to_next_speed_update -= written;
                total_written += written;

                if written < output_chunk.len() {
                    break; // input exhausted
                }
            }
        } else {
            // write into buffer without pitch changes
            self.file_source.samples_to_next_speed_update = 0;
            total_written = self.write_buffer(output);
        }

        // apply volume fading
        self.file_source
            .volume_fader
            .process(&mut output[..total_written]);

        // send Position change events, if needed
        let is_start_event = false;
        self.file_source.send_playback_position_status(
            time,
            is_start_event,
            self.playback_pos as u64,
            self.file_buffer.channel_count(),
            self.file_buffer.sample_rate(),
        );

        // check if we've finished playing and send Stopped events
        let fade_out_completed = self.file_source.volume_fader.state() == FaderState::Finished
            && self.file_source.volume_fader.target_volume() == 0.0;
        if self.playback_pos_eof || fade_out_completed {
            // mark playback as finished
            self.file_source
                .send_playback_stopped_status(self.playback_pos_eof);
            self.file_source.playback_finished = true;
        }

        total_written
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::source::resampled::ResamplingQuality;

    #[test]
    fn resampling() {
        let source_sample_rate = 44100;
        let target_sample_rate = 48000;

        // NB add extra tailing 0.0 sample for the cubic resampler
        let file_buffer = Arc::new(
            AudioFileBuffer::new(vec![0.2, 1.0, 0.5, 0.0], 1, source_sample_rate, None).unwrap(),
        );

        // Default
        let mut preloaded = PreloadedFileSource::from_shared_buffer(
            Arc::clone(&file_buffer),
            "buffer",
            FilePlaybackOptions::default().resampling_quality(ResamplingQuality::Default),
            target_sample_rate,
        )
        .unwrap();
        let mut output = vec![0.0; 1024];
        let written = preloaded.write(&mut output, &SourceTime::default());
        let expected_output =
            file_buffer.buffer().len() as u32 * source_sample_rate / target_sample_rate;
        assert!(written as u32 >= expected_output);
        assert!(
            (output.iter().sum::<f32>() - file_buffer.buffer().iter().sum::<f32>()).abs() < 0.1
        );

        // HighQuality
        let file_buffer =
            Arc::new(AudioFileBuffer::new(vec![0.2, 1.0, 0.5], 1, 48000, None).unwrap());

        let mut preloaded = PreloadedFileSource::from_shared_buffer(
            Arc::clone(&file_buffer),
            "buffer",
            FilePlaybackOptions::default().resampling_quality(ResamplingQuality::HighQuality),
            target_sample_rate,
        )
        .unwrap();
        let mut output = vec![0.0; 1024];
        let written = preloaded.write(&mut output, &SourceTime::default());
        let expected_output =
            file_buffer.buffer().len() as u32 * source_sample_rate / target_sample_rate;
        assert!(written as u32 >= expected_output);
        assert!(
            (output.iter().sum::<f32>() - file_buffer.buffer().iter().sum::<f32>()).abs() < 0.2
        );
        assert!(output[3..].iter().sum::<f32>() < 0.1);
    }
}
