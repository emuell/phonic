#[cfg(feature = "assert_no_alloc")]
use assert_no_alloc::*;

use std::{
    ffi,
    rc::Rc,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use crossbeam_channel::{bounded, Receiver, Sender};

use sokol::{
    audio::{self as saudio},
    log as slog,
};

use crate::{
    error::Error,
    output::{OutputDevice, OutputSink},
    source::{empty::EmptySource, Source, SourceTime},
    utils::buffer::clear_buffer,
};

// -------------------------------------------------------------------------------------------------

const PREFERRED_SAMPLE_RATE: i32 = 44100;
const PREFERRED_CHANNELS: i32 = 2;
const PREFERRED_BUFFER_SIZE: i32 = if cfg!(debug_assertions) { 4096 } else { 2048 };

// -------------------------------------------------------------------------------------------------

enum CallbackMessage {
    PlaySource(Box<dyn Source>),
    Pause,
    Resume,
    SetVolume(f32),
}

enum CallbackState {
    Playing,
    Paused,
}

struct SokolContext {
    callback_recv: Receiver<CallbackMessage>,
    source: Box<dyn Source>,
    state: CallbackState,
    playback_pos: Arc<AtomicU64>,
    playback_pos_instant: Instant,
    volume: f32,
}

// -------------------------------------------------------------------------------------------------

/// Stores a boxed SokolContext as raw ptr and ensures the box is dropped correctly.
/// Also shuts down audio before the context gets dropped.
#[derive(Debug)]
struct SokolContextRef {
    context: *mut SokolContext,
}

impl SokolContextRef {
    fn new(context: Box<SokolContext>) -> Self {
        Self {
            context: Box::into_raw(context),
        }
    }
}

impl Drop for SokolContextRef {
    fn drop(&mut self) {
        SokolOutput::audio_shutdown();
        drop(unsafe { Box::from_raw(self.context) });
    }
}

// -------------------------------------------------------------------------------------------------

// OutputSink for Sokol audio output
#[derive(Debug, Clone)]
pub struct SokolSink {
    volume: f32,
    playback_pos: Arc<AtomicU64>,
    callback_send: Sender<CallbackMessage>,
    #[allow(dead_code)]
    context_ref: Rc<SokolContextRef>,
}

impl OutputSink for SokolSink {
    fn suspended(&self) -> bool {
        saudio::suspended()
    }

    fn channel_count(&self) -> usize {
        assert!(
            saudio::isvalid(),
            "audio not yet initialized or already shut down"
        );
        saudio::channels() as usize
    }

    fn sample_rate(&self) -> u32 {
        assert!(
            saudio::isvalid(),
            "audio not yet initialized or already shut down"
        );
        saudio::sample_rate() as u32
    }

    fn sample_position(&self) -> u64 {
        self.playback_pos.load(Ordering::Relaxed)
    }

    fn volume(&self) -> f32 {
        self.volume
    }
    fn set_volume(&mut self, volume: f32) {
        self.volume = volume;
        self.callback_send
            .send(CallbackMessage::SetVolume(volume))
            .unwrap();
    }

    fn play(&mut self, source: impl Source) {
        // ensure source has our sample rate and channel layout
        assert_eq!(source.channel_count(), self.channel_count());
        assert_eq!(source.sample_rate(), self.sample_rate());
        // send message to activate it in the writer
        self.callback_send
            .send(CallbackMessage::PlaySource(Box::new(source)))
            .unwrap()
    }

    fn pause(&mut self) {
        self.callback_send.send(CallbackMessage::Pause).unwrap();
    }

    fn resume(&mut self) {
        self.callback_send.send(CallbackMessage::Resume).unwrap();
    }

    fn stop(&mut self) {
        self.callback_send
            .send(CallbackMessage::PlaySource(Box::new(EmptySource)))
            .unwrap();
    }

    fn close(&mut self) {
        self.stop();
    }
}

unsafe impl Send for SokolSink {}

// -------------------------------------------------------------------------------------------------

/// Audio output impl using the sokol audio player.
/// Creates a sink on open and manages sokol audio state.
pub struct SokolOutput {
    sink: SokolSink,
}

impl SokolOutput {
    pub fn open() -> Result<Self, Error> {
        let (callback_send, callback_recv) = bounded(16);

        let playback_pos = Arc::new(AtomicU64::new(0));

        let context = Box::new(SokolContext {
            callback_recv,
            source: Box::new(EmptySource),
            playback_pos: Arc::clone(&playback_pos),
            playback_pos_instant: Instant::now(),
            state: CallbackState::Paused,
            volume: 1.0,
        });

        let context_ref = Rc::new(SokolContextRef::new(context));

        Self::audio_init(context_ref.context);

        let sink = SokolSink {
            volume: 1.0,
            playback_pos,
            callback_send,
            context_ref,
        };

        Ok(Self { sink })
    }

    fn audio_init(context: *mut SokolContext) {
        saudio::setup(&saudio::Desc {
            stream_userdata_cb: Some(Self::audio_callback),
            user_data: context as *mut ffi::c_void,
            num_channels: PREFERRED_CHANNELS,
            buffer_frames: PREFERRED_BUFFER_SIZE,
            sample_rate: PREFERRED_SAMPLE_RATE,
            logger: saudio::Logger {
                func: Some(slog::slog_func),
                ..Default::default()
            },
            ..Default::default()
        });
    }

    fn audio_shutdown() {
        if saudio::isvalid() {
            saudio::shutdown();
        }
    }

    extern "C" fn audio_callback(
        raw_buffer: *mut f32,
        num_frames: i32,
        num_channels: i32,
        userdata: *mut ffi::c_void,
    ) {
        let state = unsafe { &mut *(userdata as *mut SokolContext) };

        // Process any pending data messages.
        while let Ok(msg) = state.callback_recv.try_recv() {
            match msg {
                CallbackMessage::PlaySource(src) => {
                    state.source = src;
                }
                CallbackMessage::Pause => {
                    state.state = CallbackState::Paused;
                }
                CallbackMessage::Resume => {
                    state.state = CallbackState::Playing;
                }
                CallbackMessage::SetVolume(volume) => {
                    state.volume = volume;
                }
            }
        }

        let output_samples = num_frames as usize * num_channels as usize;
        let output = unsafe { std::slice::from_raw_parts_mut(raw_buffer, output_samples) };

        // Write out as many samples as possible from the audio source to the output buffer.
        let samples_written = match state.state {
            CallbackState::Playing => {
                let time = SourceTime {
                    pos_in_frames: state.playback_pos.load(Ordering::Relaxed)
                        / state.source.channel_count() as u64,
                    pos_instant: state.playback_pos_instant,
                };
                #[cfg(not(feature = "assert_no_alloc"))]
                {
                    state.source.write(&mut output[..output_samples], &time)
                }
                #[cfg(feature = "assert_no_alloc")]
                {
                    assert_no_alloc(|| state.source.write(&mut output[..output_samples], &time))
                }
            }
            CallbackState::Paused => 0,
        };

        // Apply volume if needed
        if state.volume != 1.0 {
            output[..samples_written].iter_mut().for_each(|s| {
                *s *= state.volume;
            });
        }

        // Mute remaining samples, if any.
        clear_buffer(&mut output[samples_written..]);

        // Move playback pos
        state
            .playback_pos
            .fetch_add(output_samples as u64, Ordering::Relaxed);
    }
}

impl OutputDevice for SokolOutput {
    type Sink = SokolSink;

    fn sink(&self) -> Self::Sink {
        self.sink.clone()
    }
}
