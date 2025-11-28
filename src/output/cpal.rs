use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

#[cfg(feature = "assert-allocs")]
use assert_no_alloc::*;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample,
};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

use crate::{
    error::Error,
    output::{AudioHostId, OutputDevice},
    source::{empty::EmptySource, Source, SourceTime},
    utils::{
        buffer::clear_buffer,
        smoothing::{apply_smoothed_gain, ExponentialSmoothedValue, SmoothedValue},
    },
};

// -------------------------------------------------------------------------------------------------

const PREFERRED_SAMPLE_FORMAT: cpal::SampleFormat = cpal::SampleFormat::F32;
const PREFERRED_SAMPLE_RATE: cpal::SampleRate = cpal::SampleRate(44100);
const PREFERRED_CHANNELS: cpal::ChannelCount = 2;
const PREFERRED_BUFFER_SIZE: cpal::BufferSize = if cfg!(debug_assertions) {
    cpal::BufferSize::Default
} else {
    cpal::BufferSize::Fixed(2048)
};

// -------------------------------------------------------------------------------------------------

/// Audio output device impl using [cpal](https://github.com/RustAudio/cpal).
pub struct CpalOutput {
    is_running: bool,
    channel_count: cpal::ChannelCount,
    sample_rate: cpal::SampleRate,
    volume: f32,
    playback_pos: Arc<AtomicU64>,
    callback_sender: SyncSender<CallbackMessage>,
    stream_sender: SyncSender<StreamMessage>,
    #[allow(unused)]
    stream_handle: StreamThreadHandle,
}

impl CpalOutput {
    pub fn open() -> Result<Self, Error> {
        Self::open_with_host(AudioHostId::Default)
    }

    pub fn open_with_host(hostid: AudioHostId) -> Result<Self, Error> {
        let host = match hostid {
            AudioHostId::Default => cpal::default_host(),
            #[cfg(target_os = "windows")]
            AudioHostId::Asio => cpal::host_from_id(cpal::HostId::Asio)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "windows")]
            AudioHostId::Wasapi => cpal::host_from_id(cpal::HostId::Wasapi)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "linux")]
            AudioHostId::Alsa => cpal::host_from_id(cpal::HostId::Alsa)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "linux")]
            AudioHostId::Jack => cpal::host_from_id(cpal::HostId::Jack)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
        };

        // Open the default output device.
        let device = host
            .default_output_device()
            .ok_or(cpal::DefaultStreamConfigError::DeviceNotAvailable)?;

        if let Ok(name) = device.name() {
            log::info!("Using audio device: {name}");
        }

        // Get the default device config, so we know what sample format and sample rate
        // the device supports.
        let supported = Self::preferred_output_config(&device)?;
        // Shared playback position counter
        let playback_pos = Arc::new(AtomicU64::new(0));

        // default volume
        let volume = 1.0;

        // channel to send and receive callback messages
        const MESSAGE_QUEUE_SIZE: usize = 16;
        let (callback_sender, callback_receiver) = sync_channel(MESSAGE_QUEUE_SIZE);

        let stream_handle = Stream::spawn({
            let config = cpal::StreamConfig {
                buffer_size: PREFERRED_BUFFER_SIZE,
                ..supported.config()
            };
            let sample_format = supported.sample_format();
            let playback_pos = Arc::clone(&playback_pos);
            move |stream_sender| {
                Stream::open(
                    device,
                    config,
                    sample_format,
                    playback_pos,
                    volume,
                    callback_receiver,
                    stream_sender,
                )
                .expect("Failed to open audio stream")
            }
        });

        let is_running = false;
        let channel_count = supported.channels();
        let sample_rate = supported.sample_rate();
        let stream_sender = stream_handle.sender();

        Ok(Self {
            is_running,
            channel_count,
            sample_rate,
            volume,
            playback_pos,
            stream_sender,
            callback_sender,
            stream_handle,
        })
    }

    fn preferred_output_config(
        device: &cpal::Device,
    ) -> Result<cpal::SupportedStreamConfig, Error> {
        for s in device.supported_output_configs()? {
            let rates = s.min_sample_rate()..=s.max_sample_rate();
            if s.channels() == PREFERRED_CHANNELS
                && s.sample_format() == PREFERRED_SAMPLE_FORMAT
                && rates.contains(&PREFERRED_SAMPLE_RATE)
            {
                return Ok(s.with_sample_rate(PREFERRED_SAMPLE_RATE));
            }
        }

        Ok(device.default_output_config()?)
    }

    fn send_to_callback(&self, msg: CallbackMessage) {
        if let Err(err) = self.callback_sender.send(msg) {
            log::error!("Failed to send callback message: {err}");
        }
    }

    fn send_to_stream(&self, msg: StreamMessage) {
        if let Err(err) = self.stream_sender.send(msg) {
            log::error!("Failed to send stream message: {err}");
        }
    }
}

impl OutputDevice for CpalOutput {
    fn channel_count(&self) -> usize {
        self.channel_count as usize
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate.0
    }

    fn sample_position(&self) -> u64 {
        self.playback_pos.load(Ordering::Relaxed)
    }

    fn volume(&self) -> f32 {
        self.volume
    }
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.send_to_callback(CallbackMessage::SetVolume(volume));
    }

    fn is_suspended(&self) -> bool {
        false
    }

    fn is_running(&self) -> bool {
        self.is_running
    }
    fn pause(&mut self) {
        self.is_running = false;
        self.send_to_stream(StreamMessage::Pause);
        self.send_to_callback(CallbackMessage::Pause);
    }

    fn resume(&mut self) {
        self.send_to_stream(StreamMessage::Resume);
        self.send_to_callback(CallbackMessage::Resume);
        self.is_running = true;
    }

    fn play(&mut self, source: Box<dyn Source>) {
        // ensure source has our sample rate and channel layout
        assert_eq!(source.channel_count(), self.channel_count());
        assert_eq!(source.sample_rate(), self.sample_rate());
        // send message to activate it in the writer
        self.send_to_callback(CallbackMessage::PlaySource(source));
        // auto-start with the first set source
        if !self.is_running {
            self.resume();
        }
    }

    fn stop(&mut self) {
        self.send_to_callback(CallbackMessage::PlaySource(Box::new(EmptySource)));
    }

    fn close(&mut self) {
        self.send_to_stream(StreamMessage::Close);
    }
}

// -------------------------------------------------------------------------------------------------

struct StreamThreadHandle {
    sender: SyncSender<StreamMessage>,
    #[allow(dead_code)]
    thread: JoinHandle<()>,
}

impl StreamThreadHandle {
    pub fn sender(&self) -> SyncSender<StreamMessage> {
        self.sender.clone()
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(PartialEq)]
enum StreamMessage {
    Pause,
    Resume,
    Close,
}

// -------------------------------------------------------------------------------------------------

enum CallbackMessage {
    PlaySource(Box<dyn Source>),
    SetVolume(f32),
    Pause,
    Resume,
}

// -------------------------------------------------------------------------------------------------

#[derive(PartialEq)]
enum CallbackState {
    Playing,
    Paused,
}

// -------------------------------------------------------------------------------------------------

struct Stream {
    stream: cpal::Stream,
    // keep device alive with the stream
    #[allow(dead_code)]
    device: cpal::Device,
}

impl Stream {
    fn open(
        device: cpal::Device,
        config: cpal::StreamConfig,
        sample_format: cpal::SampleFormat,
        playback_pos: Arc<AtomicU64>,
        volume: f32,
        callback_receiver: Receiver<CallbackMessage>,
        stream_sender: SyncSender<StreamMessage>,
    ) -> Result<Self, Error> {
        let mut callback = StreamCallback {
            stream_sender,
            callback_receiver,
            source: Box::new(EmptySource),
            playback_pos,
            playback_pos_instant: Instant::now(),
            temp_buffer: Vec::with_capacity(StreamCallback::required_buffer_size(
                sample_format,
                &config,
            )),
            state: CallbackState::Paused,
            volume: ExponentialSmoothedValue::new(volume, config.sample_rate.0),
        };

        log::info!("Opening output stream: {:?}", &config);
        let stream = match sample_format {
            cpal::SampleFormat::I8 => {
                Self::build_output_stream::<i8, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::I16 => {
                Self::build_output_stream::<i16, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::I32 => {
                Self::build_output_stream::<i32, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::I64 => {
                Self::build_output_stream::<i64, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::U8 => {
                Self::build_output_stream::<u8, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::U16 => {
                Self::build_output_stream::<u16, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::U32 => {
                Self::build_output_stream::<u32, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::U64 => {
                Self::build_output_stream::<u64, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            cpal::SampleFormat::F32 => {
                Self::build_output_stream::<f32, _>(&device, &config, move |output| {
                    callback.write_samples_f32(output) // use specialized write function
                })
            }
            cpal::SampleFormat::F64 => {
                Self::build_output_stream::<f64, _>(&device, &config, move |output| {
                    callback.write_samples(output)
                })
            }
            sample_format => panic!("Unsupported/unexpected sample format '{sample_format}'"),
        }?;

        Ok(Self { device, stream })
    }

    pub fn build_output_stream<T, F>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut writer: F,
    ) -> Result<cpal::Stream, cpal::BuildStreamError>
    where
        T: cpal::SizedSample,
        F: FnMut(&mut [T]) + Send + 'static,
    {
        device.build_output_stream(
            config,
            move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
                writer(output);
            },
            |err| {
                log::error!("Audio output error: {err}");
            },
            None,
        )
    }

    fn spawn<F>(factory: F) -> StreamThreadHandle
    where
        F: FnOnce(SyncSender<StreamMessage>) -> Self + Send + 'static,
    {
        const MESSAGE_QUEUE_SIZE: usize = 32;
        let (send, receiver) = sync_channel(MESSAGE_QUEUE_SIZE);
        StreamThreadHandle {
            sender: send.clone(),
            thread: thread::Builder::new()
                .name("audio_output".to_string())
                .spawn(move || {
                    let this = factory(send);
                    this.process_messages(receiver);
                })
                .expect("failed to spawn audio thread"),
        }
    }

    fn process_messages(self, receiver: Receiver<StreamMessage>) {
        while let Ok(msg) = receiver.recv() {
            match msg {
                StreamMessage::Pause => {
                    log::debug!("Pausing audio output stream...");
                    if let Err(err) = self.stream.pause() {
                        log::error!("Failed to stop stream: {err}");
                    }
                }
                StreamMessage::Resume => {
                    log::debug!("Resuming audio output stream...");
                    if let Err(err) = self.stream.play() {
                        log::error!("Failed to start stream: {err}");
                    }
                }
                StreamMessage::Close => {
                    log::debug!("Closing audio output stream...");
                    if let Err(err) = self.stream.pause() {
                        log::error!("Failed to pause stream before stopping: {err}");
                    }
                    break;
                }
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

struct StreamCallback {
    #[allow(dead_code)]
    stream_sender: SyncSender<StreamMessage>,
    callback_receiver: Receiver<CallbackMessage>,
    source: Box<dyn Source>,
    playback_pos: Arc<AtomicU64>,
    playback_pos_instant: Instant,
    temp_buffer: Vec<f32>,
    state: CallbackState,
    volume: ExponentialSmoothedValue,
}

impl StreamCallback {
    fn required_buffer_size(
        sample_format: cpal::SampleFormat,
        config: &cpal::StreamConfig,
    ) -> usize {
        if sample_format != cpal::SampleFormat::F32 {
            let max_frames = match config.buffer_size {
                cpal::BufferSize::Default => 2048,
                cpal::BufferSize::Fixed(fixed) => fixed,
            };
            max_frames as usize * config.channels as usize
        } else {
            0 // no temp buffer needed with write_samples_f32
        }
    }

    fn write_samples_f32(&mut self, output: &mut [f32]) {
        // Handle messages
        self.process_messages();
        // Avoid temp buffers and write directly into the given buffer
        let written = self.write_source(output);
        // Clear remaining output
        clear_buffer(&mut output[written..]);
    }

    fn write_samples<T>(&mut self, output: &mut [T])
    where
        T: cpal::SizedSample + cpal::FromSample<f32>,
    {
        // Handle messages
        self.process_messages();
        // Temporarily take ownership of the output buffer so we avoid borrowing self twice.
        let mut temp_buffer = std::mem::take(&mut self.temp_buffer);
        temp_buffer.resize(output.len(), 0.0);
        // Write into the f32 temp buffer
        let written = self.write_source(&mut temp_buffer);
        // Convert from f32 to the target sample type
        for (o, i) in output.iter_mut().zip(temp_buffer.iter()).take(written) {
            *o = i.to_sample();
        }
        // Clear remaining output
        for o in &mut output[written..] {
            *o = T::EQUILIBRIUM;
        }
        // Give the temp buffer back to self.
        self.temp_buffer = temp_buffer;
    }

    fn process_messages(&mut self) {
        // Process any pending data messages.
        while let Ok(msg) = self.callback_receiver.try_recv() {
            match msg {
                CallbackMessage::PlaySource(src) => {
                    self.source = src;
                }
                CallbackMessage::SetVolume(volume) => {
                    self.volume.set_target(volume);
                }
                CallbackMessage::Pause => {
                    self.state = CallbackState::Paused;
                }
                CallbackMessage::Resume => {
                    self.state = CallbackState::Playing;
                }
            }
        }
    }

    fn write_source(&mut self, output: &mut [f32]) -> usize {
        // Only run the source when playing
        if self.state != CallbackState::Playing {
            return 0;
        }
        // Calculate source time from playback position
        let time = SourceTime {
            pos_in_frames: self.playback_pos.load(Ordering::Relaxed)
                / self.source.channel_count().max(1) as u64,
            pos_instant: self.playback_pos_instant,
        };
        // Write out as many samples as possible from the audio source to the output buffer.
        #[cfg(not(feature = "assert-allocs"))]
        let written = self.source.write(output, &time);
        #[cfg(feature = "assert-allocs")]
        let written = assert_no_alloc(|| self.source.write(output, &time));
        // Apply the global volume level
        apply_smoothed_gain(&mut output[..written], &mut self.volume);
        // Advance playback pos
        self.playback_pos
            .fetch_add(output.len() as u64, Ordering::Relaxed);
        // return modified samples
        written
    }
}

// -------------------------------------------------------------------------------------------------

impl From<cpal::DefaultStreamConfigError> for Error {
    fn from(err: cpal::DefaultStreamConfigError) -> Error {
        Error::OutputDeviceError(Box::new(err))
    }
}

impl From<cpal::SupportedStreamConfigsError> for Error {
    fn from(err: cpal::SupportedStreamConfigsError) -> Error {
        Error::OutputDeviceError(Box::new(err))
    }
}

impl From<cpal::BuildStreamError> for Error {
    fn from(err: cpal::BuildStreamError) -> Error {
        Error::OutputDeviceError(Box::new(err))
    }
}

impl From<cpal::PlayStreamError> for Error {
    fn from(err: cpal::PlayStreamError) -> Error {
        Error::OutputDeviceError(Box::new(err))
    }
}

impl From<cpal::PauseStreamError> for Error {
    fn from(err: cpal::PauseStreamError) -> Error {
        Error::OutputDeviceError(Box::new(err))
    }
}
