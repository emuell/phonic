use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

#[cfg(feature = "assert_no_alloc")]
use assert_no_alloc::*;

use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    Sample,
};
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::{
    error::Error,
    output::{AudioHostId, OutputDevice, OutputSink},
    source::{empty::EmptySource, Source, SourceTime},
    utils::{
        actor::{Act, Actor, ActorHandle},
        buffer::{clear_buffer, scale_buffer},
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

pub struct CpalOutput {
    _handle: ActorHandle<StreamMsg>,
    sink: CpalSink,
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
            log::info!("using audio device: {}", name);
        }

        // Get the default device config, so we know what sample format and sample rate
        // the device supports.
        let supported = Self::preferred_output_config(&device)?;
        // Shared playback position counter
        let playback_pos = Arc::new(AtomicU64::new(0));

        let (callback_send, callback_recv) = bounded(16);

        let handle = Stream::spawn_with_default_cap("audio_output", {
            let config = cpal::StreamConfig {
                buffer_size: PREFERRED_BUFFER_SIZE,
                ..supported.config()
            };
            let sample_format = supported.sample_format();
            let playback_pos = Arc::clone(&playback_pos);
            move |this| {
                Stream::open(
                    device,
                    config,
                    sample_format,
                    playback_pos,
                    callback_recv,
                    this,
                )
                .expect("Failed to open audio stream")
            }
        });
        let sink = CpalSink {
            channel_count: supported.channels(),
            sample_rate: supported.sample_rate(),
            volume: 1.0,
            playback_pos,
            stream_send: handle.sender(),
            callback_send,
        };

        Ok(Self {
            _handle: handle,
            sink,
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
}

impl OutputDevice for CpalOutput {
    type Sink = CpalSink;

    fn sink(&self) -> Self::Sink {
        self.sink.clone()
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct CpalSink {
    channel_count: cpal::ChannelCount,
    sample_rate: cpal::SampleRate,
    volume: f32,
    playback_pos: Arc<AtomicU64>,
    callback_send: Sender<CallbackMsg>,
    stream_send: Sender<StreamMsg>,
}

impl CpalSink {
    fn send_to_callback(&self, msg: CallbackMsg) {
        if self.callback_send.send(msg).is_err() {
            log::error!("output stream actor is dead");
        }
    }

    fn send_to_stream(&self, msg: StreamMsg) {
        if self.stream_send.send(msg).is_err() {
            log::error!("output stream actor is dead");
        }
    }
}

impl OutputSink for CpalSink {
    fn suspended(&self) -> bool {
        false
    }

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
        self.send_to_callback(CallbackMsg::SetVolume(volume));
    }

    fn play(&mut self, source: impl Source) {
        // ensure source has our sample rate and channel layout
        assert_eq!(source.channel_count(), self.channel_count());
        assert_eq!(source.sample_rate(), self.sample_rate());
        // send message to activate it in the writer
        self.send_to_callback(CallbackMsg::PlaySource(Box::new(source)));
    }

    fn pause(&mut self) {
        self.send_to_stream(StreamMsg::Pause);
        self.send_to_callback(CallbackMsg::Pause);
    }

    fn resume(&mut self) {
        self.send_to_stream(StreamMsg::Resume);
        self.send_to_callback(CallbackMsg::Resume);
    }

    fn stop(&mut self) {
        self.send_to_callback(CallbackMsg::PlaySource(Box::new(EmptySource)));
    }

    fn close(&mut self) {
        self.send_to_stream(StreamMsg::Close);
    }
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
        callback_recv: Receiver<CallbackMsg>,
        stream_send: Sender<StreamMsg>,
    ) -> Result<Self, Error> {
        let mut callback = StreamCallback {
            stream_send,
            callback_recv,
            source: Box::new(EmptySource),
            playback_pos,
            playback_pos_instant: Instant::now(),
            temp_buffer: Vec::with_capacity(StreamCallback::required_buffer_size(
                sample_format,
                &config,
            )),
            state: CallbackState::Paused,
            volume: 1.0,
        };
        log::info!("opening output stream: {:?}", &config);
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
                log::error!("audio output error: {}", err);
            },
            None,
        )
    }
}

impl Actor for Stream {
    type Message = StreamMsg;
    type Error = Error;

    fn handle(&mut self, msg: Self::Message) -> Result<Act<Self>, Self::Error> {
        match msg {
            StreamMsg::Pause => {
                log::debug!("pausing audio output stream");
                if let Err(err) = self.stream.pause() {
                    log::error!("failed to stop stream: {}", err);
                }
                Ok(Act::Continue)
            }
            StreamMsg::Resume => {
                log::debug!("resuming audio output stream");
                if let Err(err) = self.stream.play() {
                    log::error!("failed to start stream: {}", err);
                }
                Ok(Act::Continue)
            }
            StreamMsg::Close => {
                log::debug!("closing audio output stream");
                let _ = self.stream.pause();
                Ok(Act::Shutdown)
            }
        }
    }
}

// -------------------------------------------------------------------------------------------------

#[derive(PartialEq)]
enum StreamMsg {
    Pause,
    Resume,
    Close,
}

enum CallbackMsg {
    PlaySource(Box<dyn Source>),
    SetVolume(f32),
    Pause,
    Resume,
}

#[derive(PartialEq)]
enum CallbackState {
    Playing,
    Paused,
}

struct StreamCallback {
    #[allow(dead_code)]
    stream_send: Sender<StreamMsg>,
    callback_recv: Receiver<CallbackMsg>,
    source: Box<dyn Source>,
    playback_pos: Arc<AtomicU64>,
    playback_pos_instant: Instant,
    temp_buffer: Vec<f32>,
    state: CallbackState,
    volume: f32,
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
        while let Ok(msg) = self.callback_recv.try_recv() {
            match msg {
                CallbackMsg::PlaySource(src) => {
                    self.source = src;
                }
                CallbackMsg::SetVolume(volume) => {
                    self.volume = volume;
                }
                CallbackMsg::Pause => {
                    self.state = CallbackState::Paused;
                }
                CallbackMsg::Resume => {
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
        #[cfg(not(feature = "assert_no_alloc"))]
        let written = self.source.write(output, &time);
        #[cfg(feature = "assert_no_alloc")]
        let written = assert_no_alloc(|| self.source.write(output, &time));
        // Apply the global volume level.
        if (1.0 - self.volume).abs() > 0.0001 {
            scale_buffer(&mut output[..written], self.volume);
        }
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
