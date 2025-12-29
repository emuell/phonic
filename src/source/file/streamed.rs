use std::{
    ops::Range,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use audio_thread_priority::promote_current_thread_to_real_time;
use crossbeam_queue::ArrayQueue;
use rb::{Consumer, Producer, RbConsumer, RbProducer, SpscRb, RB};
use symphonia::core::audio::{SampleBuffer, SignalSpec};

use super::{
    common::FileSourceImpl, decoder::AudioDecoder, FilePlaybackMessage, FilePlaybackOptions,
    FileSource,
};

use crate::{
    error::Error,
    player::PlaybackId,
    source::{
        status::{PlaybackStatusContext, PlaybackStatusEvent},
        Source, SourceTime,
    },
    utils::{buffer::clear_buffer, fader::FaderState},
};

// -------------------------------------------------------------------------------------------------

/// Events to control the decoder thread of a streamed FileSource
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StreamedFileSourceMessage {
    /// Seek the decoder to a new position
    Seek(Duration),
    /// Start reading streamed source
    Read,
    /// Stop the decoder gracefully.
    Stop,
    /// Stop the decoder by force, immediately.
    Kill,
}

// -------------------------------------------------------------------------------------------------

/// A [`FileSource`] which streams & decodes an audio file asynchronously in a worker thread.
pub struct StreamedFileSource {
    stream_thread: StreamThreadHandle,
    consumer: Consumer<f32>,
    worker_state: SharedStreamThreadState,
    signal_spec: SignalSpec,
    file_source: FileSourceImpl,
}

impl StreamedFileSource {
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // Validate options
        options.validate()?;

        // Memorize file path for progress
        let file_path = Arc::new(path.as_ref().to_string_lossy().to_string());

        // Create decoder
        let decoder = AudioDecoder::from_file(path)?;

        // Get repeat option
        let repeat = options.repeat.unwrap_or(if !decoder.loops().is_empty() {
            usize::MAX
        } else {
            0
        });

        // Gather the source signal specs
        let signal_spec = decoder.signal_spec();

        // Create a ring-buffer for the decoded samples. Worker thread is producing,
        // we are consuming in the `Source` impl.
        let buffer = StreamThread::create_buffer();
        let consumer = buffer.consumer();

        let worker_state = SharedStreamThreadState {
            // We keep track of the current play-head position by sharing an atomic sample
            // counter with the decoding worker.  Worker is setting this on seek, we are
            // incrementing on reading from the ring-buffer.
            position: Arc::new(AtomicU64::new(0)),
            // Because the `n_frames` count that Symphonia gives us can be a bit unreliable,
            // we track the total number of samples in this stream in this atomic, set when
            // the underlying decoder returns EOF.
            total_samples: Arc::new(AtomicU64::new(u64::MAX)),
            // True when worker reached EOF
            end_of_file: Arc::new(AtomicBool::new(false)),
            // False, when worked received a stop event
            is_playing: Arc::new(AtomicBool::new(true)),
            // True when we should apply a fading out instead of stopping.
            fade_out_on_stop: Arc::new(AtomicBool::new(
                if let Some(duration) = options.fade_out_duration {
                    !duration.is_zero()
                } else {
                    false
                },
            )),
            // True when the worker received a fadeout stop request
            is_fading_out: Arc::new(AtomicBool::new(false)),
        };

        // Spawn the worker and kick-start the decoding. The buffer will start filling now.
        let stream_thread = StreamThread::spawn({
            let shared_state = worker_state.clone();
            move |sender| StreamThread::new(sender, decoder, buffer, shared_state, repeat)
        });
        // Start stream thread and block until it started running
        stream_thread.sender.send(StreamedFileSourceMessage::Read)?;

        // create common data
        let file_source = FileSourceImpl::new(
            &file_path,
            options,
            signal_spec.rate,
            signal_spec.channels.count(),
            output_sample_rate,
        )?;

        Ok(Self {
            stream_thread,
            consumer,
            signal_spec,
            worker_state,
            file_source,
        })
    }

    pub(crate) fn total_samples(&self) -> Option<u64> {
        let total = self.worker_state.total_samples.load(Ordering::Relaxed);
        if total == u64::MAX {
            None
        } else {
            Some(total)
        }
    }

    pub(crate) fn written_samples(&self, position: u64) -> u64 {
        self.worker_state
            .position
            .fetch_add(position, Ordering::Relaxed)
            + position
    }

    fn process_messages(&mut self) {
        while let Some(msg) = self.file_source.playback_message_queue.pop() {
            match msg {
                FilePlaybackMessage::Seek(position) => {
                    if let Err(err) = self
                        .stream_thread
                        .sender
                        .try_send(StreamedFileSourceMessage::Seek(position))
                    {
                        log::warn!("Failed to send playback seek event: {err}")
                    }
                }
                FilePlaybackMessage::SetSpeed(speed, glide) => {
                    self.file_source.target_speed = speed;
                    self.file_source.speed_glide_rate = glide.unwrap_or(0.0);
                    if self.file_source.speed_glide_rate == 0.0 {
                        self.file_source.current_speed = speed;
                        self.file_source.update_speed(self.signal_spec.rate);
                    }
                }
                FilePlaybackMessage::Stop => {
                    if let Err(err) = self
                        .stream_thread
                        .sender
                        .try_send(StreamedFileSourceMessage::Stop)
                    {
                        log::warn!("Failed to send playback stop event: {err}")
                    }
                }
                FilePlaybackMessage::Kill => {
                    if let Err(err) = self
                        .stream_thread
                        .sender
                        .try_send(StreamedFileSourceMessage::Kill)
                    {
                        log::warn!("Failed to send playback stop event: {err}")
                    }
                }
            }
        }
    }

    fn write_buffer(&mut self, output: &mut [f32]) -> usize {
        let mut written = 0;

        let resampler = &mut *self.file_source.resampler;
        let resampler_input_buffer = &mut self.file_source.resampler_input_buffer;

        while written < output.len() {
            // fill up input buffer
            if resampler_input_buffer.is_empty() {
                // read input from ring buffer
                resampler_input_buffer.reset_range();
                let read_samples = self
                    .consumer
                    .read(resampler_input_buffer.get_mut())
                    .unwrap_or(0);
                resampler_input_buffer.set_range(0, read_samples);
                // pad with zeros if resampler has input size constrains
                let required_input_len = resampler.required_input_buffer_size().unwrap_or(0);
                if resampler_input_buffer.len() < required_input_len
                    && (read_samples != 0 || !self.worker_state.end_of_file.load(Ordering::Relaxed))
                {
                    log::warn!(
                        "File stream buffer timeout: Padding {} missing samples with silence...",
                        required_input_len - read_samples
                    );
                    resampler_input_buffer.set_range(0, required_input_len);
                    clear_buffer(&mut resampler_input_buffer.get_mut()[read_samples..]);
                }
            }
            // run resampler
            let (input_consumed, output_written) = resampler
                .process(resampler_input_buffer.get(), &mut output[written..])
                .expect("StreamedFile resampling failed");
            resampler_input_buffer.consume(input_consumed);
            written += output_written;

            if self.worker_state.end_of_file.load(Ordering::Relaxed) && output_written == 0 {
                // got no more output from file or resampler
                break;
            }
        }
        written
    }
}

impl FileSource for StreamedFileSource {
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

    fn current_frame_position(&self) -> u64 {
        self.worker_state.position.load(Ordering::Relaxed)
            / self.signal_spec.channels.count() as u64
    }

    fn total_frames(&self) -> Option<u64> {
        self.total_samples()
            .map(|samples| samples / self.signal_spec.channels.count() as u64)
    }

    fn end_of_track(&self) -> bool {
        self.file_source.playback_finished && self.worker_state.end_of_file.load(Ordering::Relaxed)
    }
}

impl Source for StreamedFileSource {
    fn write(&mut self, output: &mut [f32], time: &SourceTime) -> usize {
        // consume playback messages
        self.process_messages();

        // quickly bail out when we've finished playing
        if self.file_source.playback_finished {
            return 0;
        }

        // send Position start event, if needed
        if !self.file_source.playback_started {
            self.file_source.playback_started = false;
            let is_start_event = true;
            self.file_source.send_playback_position_status(
                time,
                is_start_event,
                self.written_samples(0),
                self.signal_spec.channels.count(),
                self.signal_spec.rate,
            );
        }

        // fetch input from our ring-buffer and resample it
        let mut total_written = 0;
        if self.file_source.current_speed != self.file_source.target_speed {
            // update pitch slide in blocks of SPEED_UPDATE_CHUNK_SIZE
            while total_written < output.len() {
                if self.file_source.samples_to_next_speed_update == 0 {
                    if self.file_source.current_speed != self.file_source.target_speed {
                        self.file_source.update_speed(self.signal_spec.rate);
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

        // start fade-out when we got signaled in our worker state to do so
        let is_fading_out = self.worker_state.is_fading_out.load(Ordering::Relaxed);
        if is_fading_out && self.file_source.volume_fader.target_volume() != 0.0 {
            self.file_source
                .volume_fader
                .start_fade_out(self.file_source.fade_out_duration.unwrap_or(Duration::ZERO));
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
            self.written_samples(total_written as u64),
            self.signal_spec.channels.count(),
            self.signal_spec.rate,
        );

        // check if playback finished and send Stopped events
        let is_playing = self.worker_state.is_playing.load(Ordering::Relaxed);
        let is_exhausted =
            total_written == 0 && self.worker_state.end_of_file.load(Ordering::Relaxed);
        let fadeout_completed =
            is_fading_out && self.file_source.volume_fader.state() == FaderState::Finished;
        if !is_playing || is_exhausted || fadeout_completed {
            // send stop message
            self.file_source.send_playback_stopped_status(is_exhausted);
            // stop our worker
            self.worker_state.is_playing.store(false, Ordering::Relaxed);
            // and stop processing
            self.file_source.playback_finished = true;
        }

        // return dirty output len
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

impl Drop for StreamedFileSource {
    fn drop(&mut self) {
        // ignore error: channel maybe already is disconnected
        self.file_source.fade_out_duration = None;
        let _ = self
            .stream_thread
            .sender
            .try_send(StreamedFileSourceMessage::Kill);
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(Debug)]
struct StreamThreadHandle {
    #[allow(unused)]
    thread: JoinHandle<()>,
    sender: SyncSender<StreamedFileSourceMessage>,
}

// -------------------------------------------------------------------------------------------------

/// Thread action of the file stream thread loop.
#[derive(Debug, Clone, Copy, PartialEq)]
enum StreamThreadAction {
    Continue,
    WaitOr {
        timeout: Duration,
        timeout_msg: StreamedFileSourceMessage,
    },
    Shutdown,
}

// -------------------------------------------------------------------------------------------------

/// Shared data of stream thread and file source thread.
#[derive(Debug, Clone)]
struct SharedStreamThreadState {
    /// Current position. We update this on seek and EOF only.
    position: Arc<AtomicU64>,
    /// Total number of samples. We set this on EOF.
    total_samples: Arc<AtomicU64>,
    /// Is the worker thread not stopped?
    is_playing: Arc<AtomicBool>,
    /// Did the worker thread played until the end of the file?
    end_of_file: Arc<AtomicBool>,
    /// True when we need to fade-out instead of abruptly stopping.
    fade_out_on_stop: Arc<AtomicBool>,
    /// True when a stop fadeout was requested.
    is_fading_out: Arc<AtomicBool>,
}

// -------------------------------------------------------------------------------------------------

/// Manages the file's stream thread.
struct StreamThread {
    /// Sending part of our own actor channel.
    sender: SyncSender<StreamedFileSourceMessage>,
    /// Decoder we are reading packets/samples from.
    decoder: AudioDecoder,
    /// Audio properties of the decoded signal.
    input_spec: SignalSpec,
    /// Sample buffer containing samples read in the last packet.
    input_packet: SampleBuffer<f32>,
    /// Ring-buffer for the output signal.
    output: SpscRb<f32>,
    /// Producing part of the output ring-buffer.
    output_producer: Producer<f32>,
    // Shared state with StreamedFileSource
    shared_state: SharedStreamThreadState,
    /// Range of samples in `resampled` that are awaiting flush into `output`.
    samples_to_write: Range<usize>,
    /// Number of samples that should be ignore when reading packages to compensate
    /// package quantized seeking
    samples_to_skip: u64,
    /// Number of samples written into the output channel.
    samples_written: u64,
    /// Are we in the middle of automatic read loop?
    is_reading: bool,
    /// Number of times we should repeat the source
    repeat: usize,
    /// Loop range in samples
    loop_range: Option<Range<u64>>,
}

impl StreamThread {
    fn create_buffer() -> SpscRb<f32> {
        const DEFAULT_BUFFER_SIZE: usize = 128 * 1024;
        SpscRb::new(DEFAULT_BUFFER_SIZE)
    }

    fn new(
        sender: SyncSender<StreamedFileSourceMessage>,
        decoder: AudioDecoder,
        output: SpscRb<f32>,
        shared_state: SharedStreamThreadState,
        repeat: usize,
    ) -> Self {
        const DEFAULT_MAX_FRAMES: u64 = 8 * 1024;

        let max_input_frames = decoder
            .codec_params()
            .max_frames_per_packet
            .unwrap_or(DEFAULT_MAX_FRAMES);

        let output_producer = output.producer();
        let input_packet = SampleBuffer::new(max_input_frames, decoder.signal_spec());

        let input_spec = decoder.signal_spec();
        let channel_count = input_spec.channels.count() as u64;

        let mut loop_range = None;
        if let Some(loop_info) = decoder.loops().first() {
            // TODO: for now we only support forward loops
            let loop_start = loop_info.start as u64 * channel_count;
            let loop_end = loop_info.end as u64 * channel_count;
            if loop_end > loop_start {
                loop_range = Some(loop_start..loop_end);
            }
        }

        let samples_written = 0;
        let samples_to_write = 0..0;
        let samples_to_skip = 0;

        let is_reading = false;

        // Promote the worker thread to audio priority to prevent buffer under-runs on high CPU usage.
        if let Err(err) = promote_current_thread_to_real_time(0, decoder.signal_spec().rate) {
            log::warn!("Failed to set file worker thread's priority to real-time: {err}");
        }

        Self {
            output_producer,
            input_packet,
            input_spec,
            decoder,
            sender,
            output,
            shared_state,
            samples_written,
            samples_to_write,
            samples_to_skip,
            is_reading,
            repeat,
            loop_range,
        }
    }

    fn spawn<F>(factory: F) -> StreamThreadHandle
    where
        F: FnOnce(SyncSender<StreamedFileSourceMessage>) -> Self + Send + 'static,
    {
        const MESSAGE_QUEUE_SIZE: usize = 64;
        let (sender, receiver) = sync_channel(MESSAGE_QUEUE_SIZE);
        StreamThreadHandle {
            sender: sender.clone(),
            thread: thread::Builder::new()
                .name("audio_file_decoder".to_string())
                .spawn(move || {
                    let this = factory(sender);
                    this.process_messages(receiver);
                })
                .expect("failed to spawn file decoder thread"),
        }
    }

    fn process_messages(mut self, receiver: Receiver<StreamedFileSourceMessage>) {
        let mut action = StreamThreadAction::Continue;
        loop {
            // sleep state
            let message = match action {
                StreamThreadAction::Continue => match receiver.recv() {
                    Ok(msg) => msg,
                    Err(_) => {
                        // channel got disconnected
                        break;
                    }
                },
                StreamThreadAction::WaitOr {
                    timeout,
                    timeout_msg,
                } => match receiver.recv_timeout(timeout) {
                    Ok(msg) => msg,
                    Err(RecvTimeoutError::Timeout) => timeout_msg,
                    Err(RecvTimeoutError::Disconnected) => {
                        // channel got disconnected
                        break;
                    }
                },
                StreamThreadAction::Shutdown => {
                    // stop on errors or shutdown requests
                    break;
                }
            };
            let result = match message {
                StreamedFileSourceMessage::Seek(time) => self.on_seek(time),
                StreamedFileSourceMessage::Read => self.on_read(),
                StreamedFileSourceMessage::Stop => self.on_stop(),
                StreamedFileSourceMessage::Kill => self.on_stop_forced(),
            };
            action = match result {
                Ok(action) => action,
                Err(err) => {
                    log::error!("File worker handler error: {err}");
                    StreamThreadAction::Shutdown
                }
            };
        }
    }

    fn on_stop(&mut self) -> Result<StreamThreadAction, Error> {
        if self.shared_state.fade_out_on_stop.load(Ordering::Relaxed) {
            // duration and fade out state will be picked up by our parent source
            self.shared_state
                .is_fading_out
                .store(true, Ordering::Relaxed);
            // keep running until fade-out completed
            if !self.is_reading {
                self.send_read_message()?;
            }
            Ok(StreamThreadAction::Continue)
        } else {
            // immediately stop reading
            self.is_reading = false;
            self.shared_state.is_playing.store(false, Ordering::Relaxed);
            Ok(StreamThreadAction::Shutdown)
        }
    }

    fn on_stop_forced(&mut self) -> Result<StreamThreadAction, Error> {
        // immediately stop reading
        self.is_reading = false;
        self.shared_state.is_playing.store(false, Ordering::Relaxed);
        Ok(StreamThreadAction::Shutdown)
    }

    fn on_seek(&mut self, time: Duration) -> Result<StreamThreadAction, Error> {
        match self.decoder.seek(time) {
            Ok(timestamp) => {
                if self.is_reading {
                    self.samples_to_write = 0..0;
                } else {
                    self.send_read_message()?;
                }
                let position = timestamp * self.input_spec.channels.count() as u64;
                self.samples_written = position;
                self.shared_state
                    .position
                    .store(position, Ordering::Relaxed);
                self.output.clear();
            }
            Err(err) => {
                log::error!("Failed to seek file: {err}");
            }
        }
        Ok(StreamThreadAction::Continue)
    }

    fn on_read(&mut self) -> Result<StreamThreadAction, Error> {
        // check if we no longer need to run the worker
        if !self.shared_state.is_playing.load(Ordering::Relaxed) {
            return Ok(StreamThreadAction::Shutdown);
        }

        // check if we need to fetch more input samples
        if self.samples_to_write.is_empty() {
            match self.decoder.read_packet(&mut self.input_packet) {
                Some(_) => {
                    self.samples_to_write = 0..self.input_packet.samples().len();
                    // shift playhead to the exact seek position if needed
                    if self.samples_to_skip > 0 {
                        let skip_now = self.samples_to_skip.min(self.samples_to_write.len() as u64);
                        self.samples_to_skip -= skip_now;
                        self.samples_to_write.start += skip_now as usize;
                    }
                }
                None => {
                    // reached EOF: apply repeat or stop
                    if self.continue_on_loop_boundary() {
                        // continue playing from loop start
                        self.send_read_message()?;
                        self.is_reading = true;
                        return Ok(StreamThreadAction::Continue);
                    } else {
                        // handle end of file
                        return Ok(StreamThreadAction::Continue);
                    }
                }
            }
        }

        if self.samples_to_write.is_empty() {
            // This can happen if a new packet was empty. try next packet...
            self.is_reading = true;
            self.send_read_message()?;
            return Ok(StreamThreadAction::Continue);
        }

        // We have samples to write.
        let samples_in_packet = &self.input_packet.samples()[self.samples_to_write.clone()];
        let mut samples_to_write_now = samples_in_packet;

        // Don't write past the loop end
        if let Some(loop_range) = &self.loop_range {
            let remaining_samples_in_loop =
                loop_range.end.saturating_sub(self.samples_written) as usize;
            if samples_to_write_now.len() > remaining_samples_in_loop {
                samples_to_write_now = &samples_to_write_now[..remaining_samples_in_loop];
            }
        }

        if let Ok(written) = self.output_producer.write(samples_to_write_now) {
            self.samples_written += written as u64;
            self.samples_to_write.start += written;

            if let Some(loop_range) = &self.loop_range {
                if self.samples_written >= loop_range.end {
                    if self.continue_on_loop_boundary() {
                        // continue playing from loop start
                    } else {
                        // reached end of file
                        return Ok(StreamThreadAction::Continue);
                    }
                }
            }
            self.is_reading = true;
            self.send_read_message()?;
            Ok(StreamThreadAction::Continue)
        } else {
            // Buffer is full. Wait a bit a try again. We also have to indicate that the
            // read loop is not running at the moment (if we receive a `Seek` while waiting,
            // we need it to explicitly kickstart reading again).
            self.is_reading = false;
            Ok(StreamThreadAction::WaitOr {
                timeout: Duration::from_millis(100),
                timeout_msg: StreamedFileSourceMessage::Read,
            })
        }
    }

    fn continue_on_loop_boundary(&mut self) -> bool {
        if self.repeat > 0 {
            // continue reading at the loop start
            if self.repeat != usize::MAX {
                self.repeat -= 1;
            }
            // seek to loop_start
            let loop_start = self.loop_range.as_ref().map(|r| r.start).unwrap_or(0);
            let seek_frames = loop_start / self.input_spec.channels.count() as u64;
            let seek_secs = seek_frames as f64 / self.input_spec.rate as f64;
            match self.decoder.seek(Duration::from_secs_f64(seek_secs)) {
                Ok(actual_frame_time) => {
                    // seeking may move to previous packet boundaries:
                    // compensate by skipping samples until we reach the desired exact time
                    if actual_frame_time < seek_frames {
                        self.samples_to_skip = (seek_frames - actual_frame_time)
                            * self.input_spec.channels.count() as u64;
                    } else {
                        self.samples_to_skip = 0;
                    }
                }
                Err(err) => {
                    log::error!("Failed to seek file: {err}");
                    return false;
                }
            }
            self.samples_written = loop_start;
            self.samples_to_write = 0..0;
            self.shared_state
                .position
                .store(loop_start, Ordering::Relaxed);
            true
        } else {
            // stop reading and mark as exhausted
            self.is_reading = false;
            self.shared_state.end_of_file.store(true, Ordering::Relaxed);
            self.shared_state
                .total_samples
                .store(self.samples_written, Ordering::Relaxed);
            false
        }
    }

    fn send_read_message(&mut self) -> Result<(), Error> {
        if let Err(err) = self.sender.try_send(StreamedFileSourceMessage::Read) {
            match err {
                TrySendError::Disconnected(_) => {
                    return Err(err.into()); // abort with error
                }
                TrySendError::Full(_) => {
                    log::warn!("Failed to send stream read message: {err}");
                }
            }
        }
        Ok(())
    }
}
