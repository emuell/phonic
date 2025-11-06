use std::{ops::Range, path::Path, sync::Arc};

use crossbeam_channel::Sender;
use crossbeam_queue::ArrayQueue;
use symphonia::core::audio::SampleBuffer;

use super::{
    common::FileSourceImpl, decoder::AudioDecoder, FilePlaybackMessage, FilePlaybackOptions,
    FileSource,
};

use crate::{
    error::Error,
    source::{
        file::{PlaybackId, PlaybackStatusContext, PlaybackStatusEvent},
        Source, SourceTime,
    },
    utils::{buffer::clear_buffer, fader::FaderState},
};

// -------------------------------------------------------------------------------------------------

/// Shared, decoded audio file buffer as used in [`PreloadedFileSource`].
#[derive(PartialEq)]
pub struct PreloadedFileBuffer {
    buffer: Vec<f32>,
    channel_count: usize,
    sample_rate: u32,
    loop_range: Option<Range<usize>>,
}

impl PreloadedFileBuffer {
    /// Create a new shared sample buffer. Returns an error if buffer properties are invalid.
    pub fn new(
        buffer: Vec<f32>,
        channel_count: usize,
        sample_rate: u32,
        loop_range: Option<Range<usize>>,
    ) -> Result<Self, Error> {
        if sample_rate == 0 {
            return Err(Error::ParameterError(
                "file buffer sample rate must be > 0".to_owned(),
            ));
        }
        if channel_count == 0 {
            return Err(Error::ParameterError(
                "file buffer channel count must be > 0".to_owned(),
            ));
        }
        if buffer.is_empty() {
            return Err(Error::ParameterError(
                "file buffer must not be empty".to_owned(),
            ));
        }
        if !buffer.len().is_multiple_of(channel_count) {
            return Err(Error::ParameterError(
                "file buffer length must be a multiple of the channel count".to_owned(),
            ));
        }
        if let Some(loop_range) = &loop_range {
            if loop_range.start >= loop_range.end || loop_range.end > buffer.len() {
                return Err(Error::ParameterError(
                    "file buffer loop range is out of bounds".to_owned(),
                ));
            }
        }
        Ok(Self {
            buffer,
            channel_count,
            sample_rate,
            loop_range,
        })
    }

    /// Access to the shared sample buffer's raw interleaved sample data.
    pub fn buffer(&self) -> &[f32] {
        &self.buffer
    }

    /// Access to the shared sample buffer's channel layout.
    pub fn channel_count(&self) -> usize {
        self.channel_count
    }

    /// Access to the shared sample buffer's sampling rate.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Access to the shared sample embedded loop points, if any.
    pub fn loop_range(&self) -> Option<Range<usize>> {
        self.loop_range.clone()
    }
}

// -------------------------------------------------------------------------------------------------

/// A buffered, clonable [`FileSource`], which decodes the entire file into a buffer before its
/// played back.
///
/// Buffers of preloaded file sources are shared (wrapped in an Arc), so cloning a source is
/// very cheap as this only copies a buffer reference and not the buffer itself. This way a file
/// can be pre-loaded once and can then be cloned and reused as often as necessary.
pub struct PreloadedFileSource {
    file_buffer: Arc<PreloadedFileBuffer>,
    file_source: FileSourceImpl,
    playback_repeat: usize,
    playback_pos: usize,
    playback_pos_eof: bool,
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

    /// Create a new preloaded file source with the given raw **encoded** file stream buffer.
    pub fn from_file_buffer(
        file_buffer: Vec<u8>,
        file_path: &str,
        playback_status_send: Option<Sender<PlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        Self::from_audio_decoder(
            AudioDecoder::from_buffer(file_buffer)?,
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
        let sample_rate = audio_decoder.signal_spec().rate;
        let channel_count = audio_decoder.signal_spec().channels.count();

        // prealloc entire buffer, when the decoder gives us a frame hint
        let buffer_capacity =
            audio_decoder.codec_params().n_frames.unwrap_or(0) as usize * channel_count + 1;
        let mut buffer = Vec::with_capacity(buffer_capacity);

        // decode the entire file into our buffer in chunks of max_frames_per_packet sizes
        let decode_buffer_capacity = audio_decoder
            .codec_params()
            .max_frames_per_packet
            .unwrap_or(16 * 1024 * channel_count as u64);
        let mut decode_buffer =
            SampleBuffer::<f32>::new(decode_buffer_capacity, audio_decoder.signal_spec());

        while audio_decoder.read_packet(&mut decode_buffer).is_some() {
            buffer.append(&mut decode_buffer.samples().to_vec());
        }
        if buffer.is_empty() {
            // TODO: should pass a proper error here
            return Err(Error::AudioDecodingError(Box::new(
                symphonia::core::errors::Error::DecodeError("failed to decode file"),
            )));
        } else {
            // add one extra empty sample at the end for the cubic resamplers
            buffer.extend(std::iter::repeat_n(0.0, channel_count));
        }

        let mut loop_range = None;
        if let Some(loop_info) = audio_decoder.loops().first() {
            // TODO: for now we only support forward loops
            let loop_start = (loop_info.start as usize * channel_count).min(buffer.len());
            let loop_end = (loop_info.end as usize * channel_count).min(buffer.len());
            if loop_end > loop_start {
                loop_range = Some(loop_start..loop_end);
            }
        }

        let file_buffer = Arc::new(PreloadedFileBuffer::new(
            buffer,
            channel_count,
            sample_rate,
            loop_range,
        )?);

        Self::from_shared_buffer(
            file_buffer,
            file_path,
            playback_status_send,
            options,
            output_sample_rate,
        )
    }

    /// Create a new preloaded file source from the given shared, **decoded** file buffer.
    pub fn from_shared_buffer(
        file_buffer: Arc<PreloadedFileBuffer>,
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
            file_buffer.sample_rate,
            file_buffer.channel_count,
            output_sample_rate,
            playback_status_send,
        )?;

        let playback_repeat = options
            .repeat
            .unwrap_or(if file_buffer.loop_range.is_some() {
                usize::MAX
            } else {
                0
            });
        let playback_pos = 0;
        let playback_pos_eof = false;

        Ok(Self {
            file_buffer,
            file_source,
            playback_repeat,
            playback_pos,
            playback_pos_eof,
        })
    }

    /// Create a copy of this preloaded source with the given playback options.
    pub fn clone(
        &self,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        let file_buffer = Arc::clone(&self.file_buffer);
        let playback_status_send = self.file_source.playback_status_send.clone();
        Self::from_shared_buffer(
            file_buffer,
            &self.file_source.file_path,
            playback_status_send,
            options,
            output_sample_rate,
        )
    }

    /// Access to the shared file buffer.
    pub fn file_buffer(&self) -> Arc<PreloadedFileBuffer> {
        Arc::clone(&self.file_buffer)
    }

    fn process_messages(&mut self) {
        while let Some(msg) = self.file_source.playback_message_queue.pop() {
            match msg {
                FilePlaybackMessage::Seek(position) => {
                    let buffer_pos = position.as_secs_f64()
                        * self.file_buffer.sample_rate as f64
                        * self.file_buffer.channel_count as f64;
                    self.playback_pos =
                        (buffer_pos as usize).clamp(0, self.file_buffer.buffer.len());
                    self.file_source.resampler.reset();
                }
                FilePlaybackMessage::SetSpeed(speed, glide) => {
                    self.file_source.samples_to_next_speed_update = 0;
                    self.file_source.target_speed = speed;
                    self.file_source.speed_glide_rate = glide.unwrap_or(0.0);
                    if self.file_source.speed_glide_rate == 0.0 {
                        self.file_source.current_speed = speed;
                        self.file_source.update_speed(self.file_buffer.sample_rate);
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

        let loop_range = self
            .file_buffer
            .loop_range
            .clone()
            .unwrap_or(0..self.file_buffer.buffer.len());

        let resampler = &mut self.file_source.resampler;
        let resampler_input_buffer = &mut self.file_source.resampler_input_buffer;

        let required_input_len = resampler.required_input_buffer_size().unwrap_or(0);

        while written < output.len() {
            // write from resampled buffer into output and apply volume
            let remaining_input_len = if self.playback_repeat > 0 {
                loop_range.end.saturating_sub(self.playback_pos)
            } else {
                self.file_buffer
                    .buffer
                    .len()
                    .saturating_sub(self.playback_pos)
            };
            let remaining_input_buffer = &self.file_buffer.buffer
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
                if self.playback_repeat > 0 {
                    if self.playback_repeat != usize::MAX {
                        self.playback_repeat -= 1;
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
        Some(self.file_buffer.buffer.len() as u64 / self.file_buffer.channel_count as u64)
    }

    fn current_frame_position(&self) -> u64 {
        self.playback_pos as u64 / self.file_source.output_channel_count as u64
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
                        self.file_source.update_speed(self.file_buffer.sample_rate);
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
        self.file_source.send_playback_position_status(
            self.playback_pos as u64,
            self.file_buffer.channel_count,
            self.file_buffer.sample_rate,
        );

        // check if we've finished playing and send Stopped events
        let fade_out_completed = self.file_source.volume_fader.state() == FaderState::Finished
            && self.file_source.volume_fader.target_volume() == 0.0;
        if self.playback_pos_eof || fade_out_completed {
            self.file_source
                .send_playback_stopped_status(self.playback_pos >= self.file_buffer.buffer.len());
            // mark playback as finished
            self.file_source.playback_finished = true;
        }

        total_written
    }

    fn channel_count(&self) -> usize {
        self.file_source.output_channel_count
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
        let source_sample_rate = 44100;
        let target_sample_rate = 48000;

        // NB add extra tailing 0.0 sample for the cubic resampler
        let file_buffer = Arc::new(
            PreloadedFileBuffer::new(vec![0.2, 1.0, 0.5, 0.0], 1, source_sample_rate, None)
                .unwrap(),
        );

        // Default
        let mut preloaded = PreloadedFileSource::from_shared_buffer(
            Arc::clone(&file_buffer),
            "buffer",
            None,
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
            Arc::new(PreloadedFileBuffer::new(vec![0.2, 1.0, 0.5], 1, 48000, None).unwrap());

        let mut preloaded = PreloadedFileSource::from_shared_buffer(
            Arc::clone(&file_buffer),
            "buffer",
            None,
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
