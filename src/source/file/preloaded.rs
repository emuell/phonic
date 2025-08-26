use std::{path::Path, sync::Arc};

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use symphonia::core::audio::SampleBuffer;

use super::{common::FileSourceImpl, FilePlaybackMessage, FilePlaybackOptions, FileSource};

use crate::{
    error::Error,
    source::{
        file::{PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
        Source, SourceTime,
    },
    utils::{buffer::clear_buffer, decoder::AudioDecoder, fader::FaderState},
};

// -------------------------------------------------------------------------------------------------

/// A buffered, clonable [`FileSource`], which decodes the entire file into a buffer before its
/// played back.
///
/// Buffers of preloaded file sources are shared (wrapped in an Arc), so cloning a source is
/// very cheap as this only copies a buffer reference and not the buffer itself. This way a file
/// can be pre-loaded once and can then be cloned and reused as often as necessary.
pub struct PreloadedFileSource {
    repeat: usize,
    buffer: Arc<Vec<f32>>,
    buffer_sample_rate: u32,
    buffer_channel_count: usize,
    buffer_pos: usize,
    file_source: FileSourceImpl,
}

impl PreloadedFileSource {
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // Memorize file path for progress
        let file_path = path.as_ref().to_string_lossy().to_string();
        Self::from_audio_decoder(
            AudioDecoder::from_file(path)?,
            &file_path,
            playback_status_send,
            options,
            output_sample_rate,
        )
    }

    /// Create a new preloaded file source with the given encoded file buffer.
    pub fn from_file_buffer(
        buffer: Vec<u8>,
        file_path: &str,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        Self::from_audio_decoder(
            AudioDecoder::from_buffer(buffer)?,
            file_path,
            playback_status_send,
            options,
            output_sample_rate,
        )
    }

    fn from_audio_decoder(
        mut audio_decoder: AudioDecoder,
        file_path: &str,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // validate options
        options.validate()?;

        // get buffer signal specs
        let buffer_sample_rate = audio_decoder.signal_spec().rate;
        let buffer_channel_count = audio_decoder.signal_spec().channels.count();

        // prealloc entire buffer, when the decoder gives us a frame hint
        let buffer_capacity =
            audio_decoder.codec_params().n_frames.unwrap_or(0) as usize * buffer_channel_count + 1;
        let mut buffer = Arc::new(Vec::with_capacity(buffer_capacity));

        // decode the entire file into our buffer in chunks of max_frames_per_packet sizes
        let decode_buffer_capacity = audio_decoder
            .codec_params()
            .max_frames_per_packet
            .unwrap_or(16 * 1024 * buffer_channel_count as u64);
        let mut decode_buffer =
            SampleBuffer::<f32>::new(decode_buffer_capacity, audio_decoder.signal_spec());

        let mut_buffer = Arc::get_mut(&mut buffer).unwrap();
        while audio_decoder.read_packet(&mut decode_buffer).is_some() {
            mut_buffer.append(&mut decode_buffer.samples().to_vec());
        }
        if buffer.is_empty() {
            // TODO: should pass a proper error here
            return Err(Error::AudioDecodingError(Box::new(
                symphonia::core::errors::Error::DecodeError("failed to decode file"),
            )));
        } else {
            // add one extra empty sample at the end for the cubic resamplers
            let mut_buffer = Arc::get_mut(&mut buffer).unwrap();
            for _ in 0..buffer_channel_count {
                mut_buffer.push(0.0);
            }
        }

        Self::from_buffer(
            buffer,
            buffer_sample_rate,
            buffer_channel_count,
            file_path,
            playback_status_send,
            options,
            output_sample_rate,
        )
    }

    /// Create a new preloaded file source with the given decoded and buffer,
    /// possibly a shared buffer from another PreloadedFileSource.
    pub fn from_buffer(
        buffer: Arc<Vec<f32>>,
        buffer_sample_rate: u32,
        buffer_channel_count: usize,
        file_path: &str,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // validate options
        options.validate()?;

        // create common data
        let file_source = FileSourceImpl::new(
            file_path,
            options,
            buffer_sample_rate,
            buffer_channel_count,
            output_sample_rate,
            playback_status_send,
        )?;

        let repeat = options.repeat;
        let buffer_pos = 0;

        Ok(Self {
            repeat,
            buffer,
            buffer_sample_rate,
            buffer_channel_count,
            buffer_pos,
            file_source,
        })
    }

    /// Create a copy of this preloaded source with the given playback options.
    pub fn clone(
        &self,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        Self::from_buffer(
            self.buffer(),
            self.buffer_sample_rate(),
            self.buffer_channel_count(),
            &self.file_source.file_path,
            self.file_source.playback_status_send.clone(),
            options,
            output_sample_rate,
        )
    }

    /// Get sample rate of our raw preloaded file's buffer
    pub fn buffer_sample_rate(&self) -> u32 {
        self.buffer_sample_rate
    }
    /// Get number of channels in our raw preloaded file's buffer
    pub fn buffer_channel_count(&self) -> usize {
        self.buffer_channel_count
    }
    /// Shared read-only access to the raw preloaded file's buffer
    pub fn buffer(&self) -> Arc<Vec<f32>> {
        Arc::clone(&self.buffer)
    }

    fn process_messages(&mut self) {
        while let Some(msg) = self.file_source.playback_message_queue.pop() {
            match msg {
                FilePlaybackMessage::Seek(position) => {
                    let buffer_pos = position.as_secs_f64()
                        * self.buffer_sample_rate as f64
                        * self.buffer_channel_count as f64;
                    self.buffer_pos = (buffer_pos as usize).clamp(0, self.buffer.len());
                    self.file_source.resampler.reset();
                }
                FilePlaybackMessage::SetSpeed(speed, glide) => {
                    self.file_source.samples_to_next_speed_update = 0;
                    self.file_source.target_speed = speed;
                    self.file_source.speed_glide_rate = glide.unwrap_or(0.0);
                    if self.file_source.speed_glide_rate == 0.0 {
                        self.file_source.current_speed = speed;
                        self.file_source.update_speed(self.buffer_sample_rate);
                    }
                }
                FilePlaybackMessage::Stop => {
                    if let Some(duration) = self.file_source.fade_out_duration {
                        if !duration.is_zero() {
                            self.file_source.volume_fader.start_fade_out(duration);
                        } else {
                            self.file_source.playback_finished = true;
                        }
                    } else {
                        self.file_source.playback_finished = true;
                    }
                }
            }
        }
    }

    fn write_buffer(&mut self, output: &mut [f32]) -> usize {
        let mut written = 0;
        while written < output.len() {
            // write from resampled buffer into output and apply volume
            let remaining_input_len = self.buffer.len() - self.buffer_pos;
            let remaining_input_buffer =
                &self.buffer[self.buffer_pos..self.buffer_pos + remaining_input_len];
            let remaining_output = &mut output[written..];
            // pad input with zeros if resampler has input size constrains (should only happen in the last process calls)
            let required_input_len = self
                .file_source
                .resampler
                .required_input_buffer_size()
                .unwrap_or(0);
            let (input_consumed, output_written) = if remaining_input_buffer.len()
                < required_input_len
            {
                self.file_source.resampler_input_buffer.reset_range();
                self.file_source
                    .resampler_input_buffer
                    .copy_from(remaining_input_buffer);
                clear_buffer(
                    &mut self.file_source.resampler_input_buffer.get_mut()[remaining_input_len..],
                );
                let (_, output_written) = self
                    .file_source
                    .resampler
                    .process(
                        self.file_source.resampler_input_buffer.get(),
                        remaining_output,
                    )
                    .expect("PreloadedFile resampling failed");
                (remaining_input_len, output_written)
            } else {
                self.file_source
                    .resampler
                    .process(remaining_input_buffer, remaining_output)
                    .expect("PreloadedFile resampling failed")
            };

            // move buffer read pos
            self.buffer_pos += input_consumed;
            written += output_written;

            // loop or stop when reaching end of file
            let end_of_file = self.buffer_pos >= self.buffer.len();
            if end_of_file {
                if self.repeat > 0 {
                    if self.repeat != usize::MAX {
                        self.repeat -= 1;
                    }
                    self.buffer_pos = 0;
                } else {
                    break;
                }
            }
            if output_written == 0 {
                // got no more output from file or resampler
                break;
            }
        }
        written
    }
}

impl FileSource for PreloadedFileSource {
    fn playback_id(&self) -> PlaybackId {
        self.file_source.file_id
    }

    fn playback_options(&self) -> &FilePlaybackOptions {
        &self.file_source.options
    }

    fn playback_message_queue(&self) -> Arc<ArrayQueue<FilePlaybackMessage>> {
        Arc::clone(&self.file_source.playback_message_queue)
    }

    fn playback_status_sender(&self) -> Option<Sender<PlaybackStatusEvent>> {
        self.file_source.playback_status_send.clone()
    }
    fn set_playback_status_sender(&mut self, sender: Option<Sender<PlaybackStatusEvent>>) {
        self.file_source.playback_status_send = sender;
    }

    fn playback_status_context(&self) -> Option<PlaybackStatusContext> {
        self.file_source.playback_status_context.clone()
    }
    fn set_playback_status_context(&mut self, context: Option<PlaybackStatusContext>) {
        self.file_source.playback_status_context = context;
    }

    fn total_frames(&self) -> Option<u64> {
        Some(self.buffer.len() as u64 / self.channel_count() as u64)
    }

    fn current_frame_position(&self) -> u64 {
        self.buffer_pos as u64 / self.channel_count() as u64
    }

    fn end_of_track(&self) -> bool {
        self.file_source.playback_finished
    }
}

impl Source for PreloadedFileSource {
    fn write(&mut self, output: &mut [f32], _time: &SourceTime) -> usize {
        // consume playback messages
        self.process_messages();

        // quickly bail out when we've finished playing
        if self.file_source.playback_finished {
            return 0;
        }

        let mut total_written = 0_usize;
        if self.file_source.current_speed != self.file_source.target_speed {
            // update pitch slide in blocks of SPEED_UPDATE_CHUNK_SIZE
            while total_written < output.len() {
                if self.file_source.samples_to_next_speed_update == 0 {
                    if self.file_source.current_speed != self.file_source.target_speed {
                        self.file_source.update_speed(self.buffer_sample_rate);
                    }
                    self.file_source.samples_to_next_speed_update =
                        FileSourceImpl::SPEED_UPDATE_CHUNK_SIZE;
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
        let position = self
            .file_source
            .samples_to_duration(self.buffer_pos as u64, self.buffer_channel_count);
        self.file_source.send_playback_position_status(position);

        // check if we've finished playing and send Stopped events
        let end_of_file = self.buffer_pos >= self.buffer.len();
        let fade_out_completed = self.file_source.volume_fader.state() == FaderState::Finished
            && self.file_source.volume_fader.target_volume() == 0.0;
        if end_of_file || fade_out_completed {
            self.file_source
                .send_playback_stopped_status(self.buffer_pos >= self.buffer.len());
            // mark playback as finished
            self.file_source.playback_finished = true;
        }

        total_written
    }

    fn channel_count(&self) -> usize {
        self.buffer_channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.file_source.output_sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.file_source.playback_finished
    }
}

// -------------------------------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::source::resampled::ResamplingQuality;

    #[test]
    fn resampling() {
        // add one extra zero sample for cubic resampling
        let buffer = Arc::new(vec![0.2, 1.0, 0.5, 0.0]);

        // Default
        let preloaded = PreloadedFileSource::from_buffer(
            buffer.clone(),
            44100,
            1,
            "temp_file",
            None,
            FilePlaybackOptions::default().resampling_quality(ResamplingQuality::Default),
            48000,
        );
        assert!(preloaded.is_ok());
        let mut preloaded = preloaded.unwrap();
        let mut output = vec![0.0; 1024];
        let written = preloaded.write(&mut output, &SourceTime::default());

        assert_eq!(written, buffer.len() - 1);
        assert!((output.iter().sum::<f32>() - buffer.iter().sum::<f32>()).abs() < 0.1);

        // Rubato
        let preloaded = PreloadedFileSource::from_buffer(
            buffer.clone(),
            44100,
            1,
            "temp_file",
            None,
            FilePlaybackOptions::default().resampling_quality(ResamplingQuality::HighQuality),
            48000,
        );
        assert!(preloaded.is_ok());
        let mut preloaded = preloaded.unwrap();
        let mut output = vec![0.0; 1024];
        let written = preloaded.write(&mut output, &SourceTime::default());

        assert!(written > buffer.len());
        assert!((output.iter().sum::<f32>() - buffer.iter().sum::<f32>()).abs() < 0.2);
        assert!(output[3..].iter().sum::<f32>() < 0.1);
    }
}
