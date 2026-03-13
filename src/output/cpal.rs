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
    output::OutputDevice,
    source::{empty::EmptySource, Source, SourceTime},
    utils::{
        buffer::clear_buffer,
        smoothing::{apply_smoothed_gain, ExponentialSmoothedValue, SmoothedValue},
    },
};

// -------------------------------------------------------------------------------------------------

/// Prefered cpal device config when using the default/auto config.
const PREFERRED_SAMPLE_FORMAT: cpal::SampleFormat = cpal::SampleFormat::F32;
const PREFERRED_SAMPLE_RATE: cpal::SampleRate = 44100;
const PREFERRED_CHANNELS: cpal::ChannelCount = 2;

// -------------------------------------------------------------------------------------------------

/// Available audio backends for [`CpalOutput`].
///
/// Represents different audio backends available on various platforms.
/// The default variant uses the system-preferred audio host.
#[cfg(feature = "cpal-output")]
#[derive(Debug, Default, Clone, Copy)]
pub enum CpalOutputDeviceDriver {
    /// System's default audio host
    #[default]
    Default,
    /// Windows: Audio Stream Input/Output (ASIO)
    #[cfg(target_os = "windows")]
    Asio,
    /// Windows: Windows Audio Session API (WASAPI)
    #[cfg(target_os = "windows")]
    Wasapi,
    /// Linux: Advanced Linux Sound Architecture
    #[cfg(target_os = "linux")]
    Alsa,
    /// macOS: CoreAudio
    #[cfg(target_os = "macos")]
    CoreAudio,
    /// Windows, Linux & macOS: JACK Audio Connection Kit
    #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
    Jack,
}

// -------------------------------------------------------------------------------------------------

/// Unique device id for a [`CpalOutput`] device.
pub type CpalDeviceId = cpal::DeviceId;

// -------------------------------------------------------------------------------------------------

/// Configuration for a [`CpalOutput`] device.
///
/// Use with [`CpalOutput::open_with_config`] to select a specific audio driver, device,
/// sample rate and buffer size from a UI or configuration file.
#[derive(Debug, Default)]
pub struct CpalOutputConfig {
    /// Audio host/driver to use. Defaults to [`CpalOutputDeviceDriver::Default`].
    pub driver: CpalOutputDeviceDriver,
    /// Id of the output device to open. `None` selects the driver's default device.
    pub device_id: Option<CpalDeviceId>,
    /// Desired sample rate in Hz. `None` uses the preferred rate (44100) or device default.
    pub sample_rate: Option<u32>,
    /// Audio buffer size in frames. `None` uses the platform default buffer size.
    pub buffer_size: Option<u32>,
}

// -------------------------------------------------------------------------------------------------

/// Audio output device impl using [cpal](https://github.com/RustAudio/cpal).
pub struct CpalOutput {
    is_running: bool,
    stream_config: cpal::StreamConfig,
    playback_pos: Arc<AtomicU64>,
    volume: f32,
    callback_sender: SyncSender<CallbackMessage>,
    stream_sender: SyncSender<StreamMessage>,
    stream_handle: StreamThreadHandle,
}

impl CpalOutput {
    /// Open an audio output device using the default configuration.
    pub fn open() -> Result<Self, Error> {
        Self::open_with_config(CpalOutputConfig::default())
    }

    /// Open an audio output device using the given configuration.
    ///
    /// Use [`CpalOutput::available_drivers`], [`CpalOutput::available_devices`] and
    /// [`CpalOutput::supported_sample_rates`] to enumerate available options dynamically.
    pub fn open_with_config(config: CpalOutputConfig) -> Result<Self, Error> {
        let host = Self::open_host(config.driver)?;

        // Find device by name or use the host default.
        let open_device = || -> Result<cpal::Device, Error> {
            if let Some(device_id) = &config.device_id {
                Ok(host
                    .output_devices()
                    .map_err(|err| Error::OutputDeviceError(Box::new(err)))?
                    .find(|d| d.id().ok().as_ref() == Some(device_id))
                    .ok_or(cpal::DefaultStreamConfigError::DeviceNotAvailable)?)
            } else {
                Ok(host
                    .default_output_device()
                    .ok_or(cpal::DefaultStreamConfigError::DeviceNotAvailable)?)
            }
        };

        let device = open_device()?;
        if let Ok(description) = device.description() {
            log::info!("Using audio device: {description}");
        }

        // Get the preferred stream config for the requested (or default) sample rate.
        let supported_stream_config = Self::select_stream_config(&device, config.sample_rate)?;

        // Shared playback position counter
        let playback_pos = Arc::new(AtomicU64::new(0));
        // Default volume
        let volume = 1.0;

        // Channel to send stream messages (pause/resume/close)
        const STREAM_MESSAGE_QUEUE_SIZE: usize = 32;
        let (stream_sender, stream_receiver) = sync_channel(STREAM_MESSAGE_QUEUE_SIZE);

        // Try opening the stream with the given buffer size
        const MESSAGE_QUEUE_SIZE: usize = 16;
        let try_open_stream = |device: cpal::Device,
                               buffer_size: Option<u32>|
         -> Result<
            (Stream, SyncSender<CallbackMessage>, cpal::StreamConfig),
            Error,
        > {
            let (callback_sender, callback_receiver) = sync_channel(MESSAGE_QUEUE_SIZE);
            let stream_config = cpal::StreamConfig {
                channels: supported_stream_config.channels(),
                sample_rate: supported_stream_config.sample_rate(),
                buffer_size: buffer_size
                    .map(cpal::BufferSize::Fixed)
                    .unwrap_or(cpal::BufferSize::Default),
            };
            let sample_format = supported_stream_config.sample_format();
            let stream = Stream::open(
                device,
                stream_config.clone(),
                sample_format,
                Arc::clone(&playback_pos),
                volume,
                callback_receiver,
                stream_sender.clone(),
            )?;
            Ok((stream, callback_sender, stream_config))
        };

        let (stream, callback_sender, stream_config) =
            match try_open_stream(device, config.buffer_size) {
                Ok(result) => result,
                Err(err) if config.buffer_size.is_some() => {
                    log::warn!(
                        "Failed to open audio stream with fixed buffer size ({err}), \
                     retrying with default buffer size..."
                    );
                    let fallback_device = open_device()?;
                    try_open_stream(fallback_device, None)?
                }
                Err(err) => return Err(err),
            };

        // Move the stream to a new thread
        let stream_handle = StreamThreadHandle {
            sender: stream_sender,
            thread: Some(
                thread::Builder::new()
                    .name("audio_output".to_string())
                    .spawn(move || stream.process_messages(stream_receiver))
                    .expect("failed to spawn audio thread"),
            ),
        };

        let is_running = false;
        let stream_sender = stream_handle.sender.clone();

        Ok(Self {
            is_running,
            stream_config,
            playback_pos,
            volume,
            stream_sender,
            callback_sender,
            stream_handle,
        })
    }

    /// Returns all audio drivers available on this platform.
    ///
    /// Always includes [`CpalOutputDeviceDriver::Default`], followed by any named drivers that are
    /// currently available (e.g. ASIO, WASAPI on Windows; ALSA, JACK on Linux).
    pub fn available_drivers() -> Vec<CpalOutputDeviceDriver> {
        let hosts = cpal::available_hosts();
        let mut drivers = vec![CpalOutputDeviceDriver::Default];
        #[cfg(target_os = "windows")]
        if hosts.contains(&cpal::HostId::Asio) {
            drivers.push(CpalOutputDeviceDriver::Asio);
        }
        #[cfg(target_os = "windows")]
        if hosts.contains(&cpal::HostId::Wasapi) {
            drivers.push(CpalOutputDeviceDriver::Wasapi);
        }
        #[cfg(target_os = "linux")]
        if hosts.contains(&cpal::HostId::Alsa) {
            drivers.push(CpalOutputDeviceDriver::Alsa);
        }
        #[cfg(target_os = "macos")]
        if hosts.contains(&cpal::HostId::CoreAudio) {
            drivers.push(CpalOutputDeviceDriver::CoreAudio);
        }
        #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
        if hosts.contains(&cpal::HostId::Jack) {
            drivers.push(CpalOutputDeviceDriver::Jack);
        }
        drivers
    }

    /// Returns `(id, name)`s of all output devices available for the given driver.
    pub fn available_devices(
        driver: CpalOutputDeviceDriver,
    ) -> Result<Vec<(cpal::DeviceId, String)>, Error> {
        let host = Self::open_host(driver)?;
        let mut devices = Vec::new();
        for device in host
            .output_devices()
            .map_err(|err| Error::OutputDeviceError(Box::new(err)))?
        {
            match (device.id(), device.description()) {
                (Ok(id), Ok(description)) => {
                    devices.push((id, description.to_string()));
                }
                (Ok(id), Err(_)) => {
                    devices.push((id.clone(), id.to_string()));
                }
                (Err(err), _) => {
                    log::warn!("Failed to query audio device id {err}")
                }
            }
        }
        Ok(devices)
    }

    /// Returns unique sample rates supported by an output device, sorted ascending.
    ///
    /// Pass `device_name = None` to query the driver's default device.
    pub fn supported_sample_rates(
        driver: CpalOutputDeviceDriver,
        device_id: Option<CpalDeviceId>,
    ) -> Result<Vec<u32>, Error> {
        let host = Self::open_host(driver)?;
        let device = if let Some(device_id) = &device_id {
            host.output_devices()
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?
                .find(|d| d.id().ok().as_ref() == Some(device_id))
                .ok_or(cpal::DefaultStreamConfigError::DeviceNotAvailable)?
        } else {
            host.default_output_device()
                .ok_or(cpal::DefaultStreamConfigError::DeviceNotAvailable)?
        };
        let mut rates: Vec<u32> = device
            .supported_output_configs()?
            .flat_map(|s| [s.min_sample_rate(), s.max_sample_rate()])
            .collect();
        rates.sort_unstable();
        rates.dedup();
        Ok(rates)
    }

    /// Returns the actual buffer size the device was opened with,
    /// or `None` if the platform default is being used.
    pub fn buffer_size(&self) -> Option<u32> {
        match self.stream_config.buffer_size {
            cpal::BufferSize::Fixed(n) => Some(n),
            cpal::BufferSize::Default => None,
        }
    }

    fn open_host(driver: CpalOutputDeviceDriver) -> Result<cpal::Host, Error> {
        Ok(match driver {
            CpalOutputDeviceDriver::Default => cpal::default_host(),
            #[cfg(target_os = "windows")]
            CpalOutputDeviceDriver::Asio => cpal::host_from_id(cpal::HostId::Asio)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "windows")]
            CpalOutputDeviceDriver::Wasapi => cpal::host_from_id(cpal::HostId::Wasapi)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "linux")]
            CpalOutputDeviceDriver::Alsa => cpal::host_from_id(cpal::HostId::Alsa)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(target_os = "macos")]
            CpalOutputDeviceDriver::CoreAudio => cpal::host_from_id(cpal::HostId::CoreAudio)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
            #[cfg(any(target_os = "windows", target_os = "linux", target_os = "macos"))]
            CpalOutputDeviceDriver::Jack => cpal::host_from_id(cpal::HostId::Jack)
                .map_err(|err| Error::OutputDeviceError(Box::new(err)))?,
        })
    }

    fn select_stream_config(
        device: &cpal::Device,
        sample_rate: Option<u32>,
    ) -> Result<cpal::SupportedStreamConfig, Error> {
        let target_rate = sample_rate.unwrap_or(PREFERRED_SAMPLE_RATE);
        // Get supported configs and sort them in terms of their priority of use as a default stream format.
        let mut configs = device.supported_output_configs()?.collect::<Vec<_>>();
        configs.sort_by(|a, b| b.cmp_default_heuristics(a));
        // Match preferred 'rate + format + channels' first, then 'rate + channels' then 'rate' only
        let supports_rate = |s: &cpal::SupportedStreamConfigRange| {
            (s.min_sample_rate()..=s.max_sample_rate()).contains(&target_rate)
        };
        let best_match = configs
            .iter()
            .find(|s| {
                supports_rate(s)
                    && s.channels() == PREFERRED_CHANNELS
                    && s.sample_format() == PREFERRED_SAMPLE_FORMAT
            })
            .or_else(|| {
                configs
                    .iter()
                    .find(|s| supports_rate(s) && s.channels() == PREFERRED_CHANNELS)
            })
            .or_else(|| configs.iter().find(|s| supports_rate(s)));
        match best_match {
            Some(s) => Ok(s.with_sample_rate(target_rate)),
            None => {
                log::warn!("Found no matching audio device config which fits the prefered one. Using the device's default config instead...");
                Ok(device.default_output_config()?)
            }
        }
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
        self.stream_config.channels as usize
    }

    fn sample_rate(&self) -> u32 {
        self.stream_config.sample_rate
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
        self.send_to_callback(CallbackMessage::PlaySource(Box::new(EmptySource::new(
            self.channel_count(),
            self.sample_rate(),
        ))));
    }

    fn close(&mut self) {
        self.send_to_stream(StreamMessage::Close);
        if let Some(handle) = self.stream_handle.thread.take() {
            let _ = handle.join();
        }
    }
}

// -------------------------------------------------------------------------------------------------

struct StreamThreadHandle {
    sender: SyncSender<StreamMessage>,
    thread: Option<JoinHandle<()>>,
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
            source: Box::new(EmptySource::new(
                config.channels as usize,
                config.sample_rate,
            )),
            playback_pos,
            playback_pos_instant: Instant::now(),
            temp_buffer: Vec::with_capacity(StreamCallback::required_buffer_size(
                sample_format,
                &config,
            )),
            state: CallbackState::Paused,
            volume: ExponentialSmoothedValue::new(volume, config.sample_rate),
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

    fn build_output_stream<T, F>(
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
                    if self.state == CallbackState::Paused {
                        self.volume.init(volume);
                    } else {
                        self.volume.set_target(volume);
                    }
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
