use std::{
    ops::Range,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use rb::{Consumer, Producer, RbConsumer, RbProducer, SpscRb, RB};
use symphonia::core::audio::{SampleBuffer, SignalSpec};

use super::{FilePlaybackMessage, FilePlaybackOptions, FileSource};
use crate::{
    error::Error,
    player::{AudioFilePlaybackId, AudioFilePlaybackStatusEvent},
    source::{resampled::ResamplingQuality, AudioSource, AudioSourceTime},
    utils::{
        actor::{Act, Actor, ActorHandle},
        buffer::TempBuffer,
        decoder::AudioDecoder,
        fader::{FaderState, VolumeFader},
        resampler::{
            cubic::CubicResampler, rubato::RubatoResampler, AudioResampler, ResamplingSpecs,
        },
        unique_usize_id,
    },
};

// -------------------------------------------------------------------------------------------------

/// A source which streams & decodes an audio file asynchromiously in a worker thread.
pub struct StreamedFileSource {
    actor: ActorHandle<FilePlaybackMessage>,
    file_id: usize,
    file_path: String,
    volume: f32,
    volume_fader: VolumeFader,
    fade_out_duration: Option<Duration>,
    consumer: Consumer<f32>,
    worker_state: SharedFileWorkerState,
    event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
    signal_spec: SignalSpec,
    resampler: Box<dyn AudioResampler>,
    resampler_input_buffer: TempBuffer,
    output_sample_rate: u32,
    playback_pos_report_instant: Instant,
    playback_pos_emit_rate: Option<Duration>,
    playback_finished: bool,
}

impl StreamedFileSource {
    pub fn new(
        file_path: &str,
        event_send: Option<Sender<AudioFilePlaybackStatusEvent>>,
        options: FilePlaybackOptions,
        output_sample_rate: u32,
    ) -> Result<Self, Error> {
        // create decoder
        let decoder = AudioDecoder::new(file_path.to_string())?;
        // Gather the source signal parameters and compute how often we should report
        // the play-head position.
        let signal_spec = decoder.signal_spec();

        // Create a ring-buffer for the decoded samples. Worker thread is producing,
        // we are consuming in the `AudioSource` impl.
        let buffer = StreamedFileWorker::default_buffer();
        let consumer = buffer.consumer();

        let worker_state = SharedFileWorkerState {
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
        let actor = StreamedFileWorker::spawn_with_default_cap("audio_decoding", {
            let shared_state = worker_state.clone();
            let repeat = options.repeat;
            move |this| StreamedFileWorker::new(this, decoder, buffer, shared_state, repeat)
        });
        actor.send(FilePlaybackMessage::Read)?;

        // create volume fader
        let mut volume_fader = VolumeFader::new(signal_spec.channels.count(), signal_spec.rate);
        if let Some(duration) = options.fade_in_duration {
            if !duration.is_zero() {
                volume_fader.start_fade_in(duration);
            }
        }

        // create resampler
        let resampler_specs = ResamplingSpecs::new(
            signal_spec.rate,
            (output_sample_rate as f64 / options.speed) as u32,
            signal_spec.channels.count(),
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

        // copy remaining options which are applied while playback
        let volume = options.volume;
        let fade_out_duration = options.fade_out_duration;
        let playback_pos_emit_rate = options.playback_pos_emit_rate;

        Ok(Self {
            actor,
            file_id,
            file_path: file_path.into(),
            volume,
            volume_fader,
            fade_out_duration,
            consumer,
            event_send,
            signal_spec,
            resampler,
            resampler_input_buffer,
            output_sample_rate,
            worker_state,
            playback_pos_report_instant: Instant::now(),
            playback_pos_emit_rate,
            playback_finished: false,
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

    fn should_report_pos(&self) -> bool {
        if let Some(report_duration) = self.playback_pos_emit_rate {
            self.playback_pos_report_instant.elapsed() >= report_duration
        } else {
            false
        }
    }

    fn samples_to_duration(&self, samples: u64) -> Duration {
        let frames = samples / self.signal_spec.channels.count() as u64;
        let seconds = frames as f64 / self.output_sample_rate as f64;
        Duration::from_secs_f64(seconds)
    }
}

impl FileSource for StreamedFileSource {
    fn playback_message_sender(&self) -> Sender<FilePlaybackMessage> {
        self.actor.sender()
    }

    fn playback_id(&self) -> AudioFilePlaybackId {
        self.file_id
    }

    fn current_frame_position(&self) -> u64 {
        self.worker_state.position.load(Ordering::Relaxed) / self.channel_count() as u64
    }

    fn total_frames(&self) -> Option<u64> {
        self.total_samples()
            .map(|samples| samples / self.channel_count() as u64)
    }

    fn end_of_track(&self) -> bool {
        self.playback_finished && self.worker_state.end_of_file.load(Ordering::Relaxed)
    }
}

impl AudioSource for StreamedFileSource {
    fn write(&mut self, output: &mut [f32], _time: &AudioSourceTime) -> usize {
        // return empty handed when playback finished
        if self.playback_finished {
            return 0;
        }
        // fetch input from our ring-buffer and resample it
        let mut written = 0;
        while written < output.len() {
            if self.resampler_input_buffer.is_empty() {
                self.resampler_input_buffer.reset_range();
                let read_samples = self
                    .consumer
                    .read(self.resampler_input_buffer.get_mut())
                    .unwrap_or(0);
                self.resampler_input_buffer.set_range(0, read_samples);

                // pad with zeros if resampler has input size constrains
                let required_input_len = self.resampler.required_input_buffer_size().unwrap_or(0);
                if self.resampler_input_buffer.len() < required_input_len
                    // stop filling up empty input buffers when we've reached the end of file
                    && (read_samples != 0
                        || !self.worker_state.end_of_file.load(Ordering::Relaxed))
                {
                    self.resampler_input_buffer.set_range(0, required_input_len);
                    for o in &mut self.resampler_input_buffer.get_mut()[read_samples..] {
                        *o = 0.0;
                    }
                }
            }
            let input = self.resampler_input_buffer.get();
            let target = &mut output[written..];
            let (input_consumed, output_written) = self
                .resampler
                .process(input, target)
                .expect("StreamedFile resampling failed");
            self.resampler_input_buffer.consume(input_consumed);
            written += output_written;
            if output_written == 0 {
                // got no more output from file or resampler
                break;
            }
        }

        // update position counters
        let position = self.written_samples(written as u64);

        // apply volume parameter
        if (1.0 - self.volume).abs() > 0.0001 {
            for o in output[0..written].as_mut() {
                *o *= self.volume;
            }
        }

        // start fade-out when this got signaled in our worker state
        let is_fading_out = self.worker_state.is_fading_out.load(Ordering::Relaxed);
        if is_fading_out && self.volume_fader.target_volume() != 0.0 {
            self.volume_fader
                .start_fade_out(self.fade_out_duration.unwrap_or(Duration::ZERO));
        }

        // apply fade-in or fade-out
        self.volume_fader.process(&mut output[0..written]);

        // send position change events
        if let Some(event_send) = &self.event_send {
            if self.should_report_pos() {
                self.playback_pos_report_instant = Instant::now();
                // NB: try_send: we want to ignore full channels on playback pos events and don't want to block
                if let Err(err) = event_send.try_send(AudioFilePlaybackStatusEvent::Position {
                    id: self.file_id,
                    path: self.file_path.clone(),
                    position: self.samples_to_duration(position),
                }) {
                    log::warn!("failed to send playback event: {}", err)
                }
            }
        }

        // check if playback finished and send Stopped events
        let is_playing = self.worker_state.is_playing.load(Ordering::Relaxed);
        let is_exhausted = written == 0 && self.worker_state.end_of_file.load(Ordering::Relaxed);
        let fadeout_completed = is_fading_out && self.volume_fader.state() == FaderState::Finished;
        if !is_playing || is_exhausted || fadeout_completed {
            // we're reached end of file or got stopped: send stop message
            if let Some(event_send) = &self.event_send {
                if let Err(err) = event_send.try_send(AudioFilePlaybackStatusEvent::Stopped {
                    id: self.file_id,
                    path: self.file_path.clone(),
                    exhausted: is_exhausted,
                }) {
                    log::warn!("failed to send playback event: {}", err)
                }
            }
            // stop our worker
            self.worker_state.is_playing.store(false, Ordering::Relaxed);
            // and stop processing
            self.playback_finished = true;
        }

        // return dirty output len
        written
    }

    fn channel_count(&self) -> usize {
        self.signal_spec.channels.count()
    }

    fn sample_rate(&self) -> u32 {
        self.output_sample_rate
    }

    fn is_exhausted(&self) -> bool {
        self.playback_finished
    }
}

impl Drop for StreamedFileSource {
    fn drop(&mut self) {
        // ignore error: channel maybe already is disconnected
        self.fade_out_duration = None;
        let _ = self.actor.send(FilePlaybackMessage::Stop);
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(Clone)]
struct SharedFileWorkerState {
    /// Current position. We update this on seek and EOF only.
    position: Arc<AtomicU64>,
    /// Total number of samples. We set this on EOF.
    total_samples: Arc<AtomicU64>,
    /// Is the worker thread not stopped?
    is_playing: Arc<AtomicBool>,
    /// Did the worker thread played until the end of the file?
    end_of_file: Arc<AtomicBool>,
    /// True when we need to fade-out instad of abruptly stopping.
    fade_out_on_stop: Arc<AtomicBool>,
    /// True when a stop fadeout was requested.
    is_fading_out: Arc<AtomicBool>,
}

// -------------------------------------------------------------------------------------------------

struct StreamedFileWorker {
    /// Sending part of our own actor channel.
    this: Sender<FilePlaybackMessage>,
    /// Decoder we are reading packets/samples from.
    input: AudioDecoder,
    /// Audio properties of the decoded signal.
    input_spec: SignalSpec,
    /// Sample buffer containing samples read in the last packet.
    input_packet: SampleBuffer<f32>,
    /// Ring-buffer for the output signal.
    output: SpscRb<f32>,
    /// Producing part of the output ring-buffer.
    output_producer: Producer<f32>,
    // Shared state with StreamedFileSource
    shared_state: SharedFileWorkerState,
    /// Range of samples in `resampled` that are awaiting flush into `output`.
    samples_to_write: Range<usize>,
    /// Number of samples written into the output channel.
    samples_written: u64,
    /// Are we in the middle of automatic read loop?
    is_reading: bool,
    /// Number of times we should repeat the source
    repeat: usize,
}

impl StreamedFileWorker {
    fn default_buffer() -> SpscRb<f32> {
        const DEFAULT_BUFFER_SIZE: usize = 128 * 1024;
        SpscRb::new(DEFAULT_BUFFER_SIZE)
    }

    fn new(
        this: Sender<FilePlaybackMessage>,
        input: AudioDecoder,
        output: SpscRb<f32>,
        shared_state: SharedFileWorkerState,
        repeat: usize,
    ) -> Self {
        const DEFAULT_MAX_FRAMES: u64 = 8 * 1024;

        let max_input_frames = input
            .codec_params()
            .max_frames_per_packet
            .unwrap_or(DEFAULT_MAX_FRAMES);

        // Promote the worker thread to audio priority to prevent buffer under-runs on high CPU usage.
        if let Err(err) =
            audio_thread_priority::promote_current_thread_to_real_time(0, input.signal_spec().rate)
        {
            log::warn!(
                "failed to set file worker thread's priority to real-time: {}",
                err
            );
        }

        Self {
            output_producer: output.producer(),
            input_packet: SampleBuffer::new(max_input_frames, input.signal_spec()),
            input_spec: input.signal_spec(),
            input,
            this,
            output,
            shared_state,
            samples_written: 0,
            samples_to_write: 0..0,
            is_reading: false,
            repeat,
        }
    }
}

impl Actor for StreamedFileWorker {
    type Message = FilePlaybackMessage;
    type Error = Error;

    fn handle(&mut self, msg: FilePlaybackMessage) -> Result<Act<Self>, Self::Error> {
        match msg {
            FilePlaybackMessage::Seek(time) => self.on_seek(time),
            FilePlaybackMessage::Read => self.on_read(),
            FilePlaybackMessage::Stop => self.on_stop(),
        }
    }
}

impl StreamedFileWorker {
    fn on_stop(&mut self) -> Result<Act<Self>, Error> {
        if self.shared_state.fade_out_on_stop.load(Ordering::Relaxed) {
            // duration and fade out state will be picked up by our parent source
            self.shared_state
                .is_fading_out
                .store(true, Ordering::Relaxed);
            // keep running until fade-out completed
            if !self.is_reading {
                self.this.send(FilePlaybackMessage::Read)?;
            }
            Ok(Act::Continue)
        } else {
            // immediately stop reading
            self.is_reading = false;
            self.shared_state.is_playing.store(false, Ordering::Relaxed);
            Ok(Act::Shutdown)
        }
    }

    fn on_seek(&mut self, time: Duration) -> Result<Act<Self>, Error> {
        match self.input.seek(time) {
            Ok(timestamp) => {
                if self.is_reading {
                    self.samples_to_write = 0..0;
                } else {
                    self.this.send(FilePlaybackMessage::Read)?;
                }
                let position = timestamp * self.input_spec.channels.count() as u64;
                self.samples_written = position;
                self.shared_state
                    .position
                    .store(position, Ordering::Relaxed);
                self.output.clear();
            }
            Err(err) => {
                log::error!("failed to seek: {}", err);
            }
        }
        Ok(Act::Continue)
    }

    fn on_read(&mut self) -> Result<Act<Self>, Error> {
        // check if we no longer need to run the worker
        if !self.shared_state.is_playing.load(Ordering::Relaxed) {
            return Ok(Act::Shutdown);
        }
        // check if we need to fetch more input samples
        if !self.samples_to_write.is_empty() {
            let input = &self.input_packet.samples()[self.samples_to_write.clone()];
            // TODO: self.output_fader.process(&mut input_mut.borrow_mut());
            if let Ok(written) = self.output_producer.write(input) {
                self.samples_written += written as u64;
                self.samples_to_write.start += written;
                self.is_reading = true;
                self.this.send(FilePlaybackMessage::Read)?;
                Ok(Act::Continue)
            } else {
                // Buffer is full.  Wait a bit a try again.  We also have to indicate that the
                // read loop is not running at the moment (if we receive a `Seek` while waiting,
                // we need it to explicitly kickstart reading again).
                self.is_reading = false;
                Ok(Act::WaitOr {
                    timeout: Duration::from_millis(500),
                    timeout_msg: FilePlaybackMessage::Read,
                })
            }
        } else {
            // fetch more input samples
            match self.input.read_packet(&mut self.input_packet) {
                Some(_) => {
                    // continue reading
                    self.samples_to_write = 0..self.input_packet.samples().len();
                    self.is_reading = true;
                    self.this.send(FilePlaybackMessage::Read)?;
                }
                None => {
                    // reached EOF
                    if self.repeat > 0 {
                        if self.repeat != usize::MAX {
                            self.repeat -= 1;
                        }
                        // seek to start and continue reading
                        self.input.seek(Duration::ZERO)?;
                        self.samples_written = 0;
                        self.samples_to_write = 0..0;
                        self.shared_state.position.store(0, Ordering::Relaxed);
                        self.is_reading = true;
                        self.this.send(FilePlaybackMessage::Read)?;
                    } else {
                        // stop reading and mark as exhausted
                        self.is_reading = false;
                        self.shared_state.end_of_file.store(true, Ordering::Relaxed);
                        self.shared_state
                            .total_samples
                            .store(self.samples_written, Ordering::Relaxed);
                    }
                }
            }
            Ok(Act::Continue)
        }
    }
}
