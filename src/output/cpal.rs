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
    StreamConfig,
};
use crossbeam_channel::{bounded, Receiver, Sender};

use crate::{
    error::Error,
    output::{AudioHostId, OutputDevice, OutputSink},
    source::{empty::EmptySource, Source, SourceTime},
    utils::actor::{Act, Actor, ActorHandle},
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
            let config = StreamConfig {
                buffer_size: PREFERRED_BUFFER_SIZE,
                ..supported.config()
            };
            let playback_pos = Arc::clone(&playback_pos);
            move |this| Stream::open(device, config, playback_pos, callback_recv, this).unwrap()
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
    _device: cpal::Device,
}

impl Stream {
    fn open(
        device: cpal::Device,
        config: cpal::StreamConfig,
        playback_pos: Arc<AtomicU64>,
        callback_recv: Receiver<CallbackMsg>,
        _stream_send: Sender<StreamMsg>,
    ) -> Result<Self, Error> {
        let mut callback = StreamCallback {
            _stream_send,
            callback_recv,
            source: Box::new(EmptySource),
            volume: 1.0,
            playback_pos,
            playback_pos_instant: Instant::now(),
            state: CallbackState::Paused,
        };

        log::info!("opening output stream: {:?}", config);
        let stream = device.build_output_stream(
            &config,
            move |output, _| {
                callback.write_samples(output);
            },
            |err| {
                log::error!("audio output error: {}", err);
            },
            None,
        )?;

        Ok(Self {
            _device: device,
            stream,
        })
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

enum CallbackState {
    Playing,
    Paused,
}

struct StreamCallback {
    _stream_send: Sender<StreamMsg>,
    callback_recv: Receiver<CallbackMsg>,
    source: Box<dyn Source>,
    playback_pos: Arc<AtomicU64>,
    playback_pos_instant: Instant,
    state: CallbackState,
    volume: f32,
}

impl StreamCallback {
    fn write_samples(&mut self, output: &mut [f32]) {
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

        let written = if matches!(self.state, CallbackState::Playing) {
            // Write out as many samples as possible from the audio source to the output buffer.
            let time = SourceTime {
                pos_in_frames: self.playback_pos.load(Ordering::Relaxed)
                    / self.source.channel_count().max(1) as u64,
                pos_instant: self.playback_pos_instant,
            };

            #[cfg(not(feature = "assert_no_alloc"))]
            let written = self.source.write(output, &time);
            #[cfg(feature = "assert_no_alloc")]
            let written = assert_no_alloc(|| self.source.write(output, &time));

            // Apply the global volume level.
            output[..written].iter_mut().for_each(|s| *s *= self.volume);

            // Advance playback pos
            self.playback_pos
                .fetch_add(output.len() as u64, Ordering::Relaxed);

            // return modified samples
            written
        } else {
            0
        };

        // Mute any remaining samples.
        output[written..].iter_mut().for_each(|s| *s = 0.0);
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
