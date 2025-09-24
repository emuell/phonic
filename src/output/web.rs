#[cfg(feature = "assert_no_alloc")]
use assert_no_alloc::*;

use std::{
    ffi,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::{
    error::Error,
    output::OutputDevice,
    source::{empty::EmptySource, Source, SourceTime},
    utils::{
        buffer::clear_buffer,
        smoothed::{apply_smoothed_gain, ExponentialSmoothedValue, SmoothedValue},
    },
};

// -------------------------------------------------------------------------------------------------

const PREFERRED_SAMPLE_RATE: i32 = 44100;
const PREFERRED_CHANNELS: i32 = 2;
const PREFERRED_BUFFER_SIZE: i32 = if cfg!(debug_assertions) {
    1024 * PREFERRED_CHANNELS
} else {
    512 * PREFERRED_CHANNELS
};

// -------------------------------------------------------------------------------------------------

// JS impl of the WebAudio backend, based on the https://github.com/floooh/sokol audio impl.
//
// This is currently using a ScriptProcessorNode callback to feed the sample data into WebAudio.
// ScriptProcessorNode has been deprecated for a while because it is running from the main thread,
// with the default initialization parameters it works 'pretty well' though. Ultimately we should
// use Audio Worklets here, which do require pthreads.
//
// The magic `js!` embedding is done via https://docs.rs/crate/emscripten_rs_sys and only works in
// rust nightly builds with `asm_experimental_arch` and `macro_metavar_expr_concat` features enabled.

#[cfg(not(target_os = "emscripten"))]
compile_error!("The 'web-output' feature currently is implemented for emscripten only.");

use emscripten_rs_sys::js;

// Setup the WebAudio context and attach a ScriptProcessorNode
js! {
    fn phonic_js_init(
        sample_rate: ffi::c_int,
        num_channels: ffi::c_int,
        buffer_size: ffi::c_int,
        pull_fn_ptr: extern "C" fn(ffi::c_int) -> *const f32
    ) -> ffi::c_int,
    {
        window.WebAudio = {
            _phonic_context: null,
            _phonic_node: null,
            _phonic_pull_fn: null
        };
        WebAudio._phonic_pull_fn = wasmTable.get(pull_fn_ptr);
        if (typeof AudioContext !== "undefined") {
            WebAudio._phonic_context = new AudioContext({
                sampleRate: sample_rate,
                latencyHint: "interactive",
            });
        }
        else {
            WebAudio._phonic_context = null;
            console.error("phonic: failed to create AudioContext");
        }
        if (WebAudio._phonic_context) {
            console.log("phonic: initializing web audio...");
            WebAudio._phonic_node = WebAudio._phonic_context.createScriptProcessor(
                buffer_size, 0, num_channels);
            console.log("phonic: web audio runs at sample rate: %s with a block size of '%s' ",
              WebAudio._phonic_context.sampleRate, WebAudio._phonic_node.bufferSize);
            WebAudio._phonic_node.onaudioprocess = (event) => {
                const num_frames = event.outputBuffer.length;
                const ptr = WebAudio._phonic_pull_fn(num_frames);
                if (ptr) {
                    const num_channels = event.outputBuffer.numberOfChannels;
                    for (let chn = 0; chn < num_channels; chn++) {
                        const chan = event.outputBuffer.getChannelData(chn);
                        for (let i = 0; i < num_frames; i++) {
                            chan[i] = HEAPF32[(ptr>>2) + ((num_channels*i)+chn)];
                        }
                    }
                }
            };
            WebAudio._phonic_node.connect(WebAudio._phonic_context.destination);

            const resume_webaudio = () => {
                if (WebAudio._phonic_context) {
                    if (WebAudio._phonic_context.state === "suspended") {
                        WebAudio._phonic_context.resume();
                    }
                }
            };
            document.addEventListener("click", resume_webaudio, {once:true});
            document.addEventListener("touchend", resume_webaudio, {once:true});
            document.addEventListener("keydown", resume_webaudio, {once:true});
            return 1;
        }
        else {
            return 0;
        }
    }
}

// Shutdown the WebContext and ScriptProcessorNode
js! {
    fn phonic_js_shutdown(),
    {
        if (WebAudio && WebAudio._phonic_context !== null) {
            console.log("phonic: shutting down web audio...");
            if (WebAudio._phonic_node) {
                WebAudio._phonic_node.disconnect();
            }
            WebAudio._phonic_context.close();
            WebAudio._phonic_context = null;
            WebAudio._phonic_node = null;
        }
    }
}

// Get the actual sample rate back from the WebAudio context
js! {
    fn phonic_js_sample_rate() -> ffi::c_int,
    {
        if (WebAudio && WebAudio._phonic_context) {
            return WebAudio._phonic_context.sampleRate;
        }
        else {
            return 0;
        }
    }
}

// Get the actual buffer size in number of frames
js! {
    fn phonic_js_buffer_frames() -> ffi::c_int,
    {
        if (WebAudio && WebAudio._phonic_node) {
            return WebAudio._phonic_node.bufferSize;
        }
        else {
            return 0;
        }
    }
}

// return 1 if the WebAudio context is currently suspended, else 0
js! {
    fn phonic_js_suspended() -> ffi::c_int,
    {
        if (WebAudio && WebAudio._phonic_context) {
            if (WebAudio._phonic_context.state === "suspended") {
                return 1;
            }
            else {
                return 0;
            }
        }
        return 0;
    }
}

// -------------------------------------------------------------------------------------------------

/// Audio [`OutputDevice`] impl using WebAudio with a ScriptProcessorNode.
///
/// Should be primally used as audio output for emscripten builds, because cpal's emscripten
/// impls are broken and no longer maintained.
#[derive(Debug)]
pub struct WebOutput {
    volume: f32,
    is_running: bool,
    playback_pos: Arc<AtomicU64>,
    callback_send: Sender<CallbackMessage>,
    #[allow(dead_code)]
    context_ref: Arc<WebContextRef>,
    channel_count: usize,
    sample_rate: u32,
}

impl WebOutput {
    pub fn open() -> Result<Self, Error> {
        if unsafe {
            phonic_js_init(
                PREFERRED_SAMPLE_RATE,
                PREFERRED_CHANNELS,
                PREFERRED_BUFFER_SIZE,
                phonic_pull,
            )
        } == 0
        {
            return Err(Error::OutputDeviceError(
                ("Failed to initialize WebAudio: ".to_owned()
                    + "Please check if your browser supports 'AudioContext's")
                    .into(),
            ));
        }

        let sample_rate = unsafe { phonic_js_sample_rate() } as u32;
        let channel_count = PREFERRED_CHANNELS as usize;
        let buffer_frames = unsafe { phonic_js_buffer_frames() } as usize;

        let (callback_send, callback_recv) = bounded(16);

        let volume = 1.0;
        let is_running = false;
        let playback_pos = Arc::new(AtomicU64::new(0));

        let mut smoothed_volume = ExponentialSmoothedValue::new(sample_rate);
        smoothed_volume.init(volume);

        let context = Box::new(WebContext {
            callback_recv,
            source: Box::new(EmptySource),
            playback_pos: Arc::clone(&playback_pos),
            playback_pos_instant: Instant::now(),
            state: CallbackState::Paused,
            smoothed_volume,
            buffer: vec![0.0; buffer_frames * channel_count],
            num_channels: channel_count,
        });

        let context_ptr = Box::into_raw(context);
        unsafe {
            WEBAUDIO_CONTEXT = context_ptr;
        }

        let context_ref = Arc::new(WebContextRef { context_ptr });

        Ok(Self {
            volume,
            is_running,
            playback_pos,
            callback_send,
            context_ref,
            channel_count,
            sample_rate,
        })
    }
}

impl OutputDevice for WebOutput {
    fn channel_count(&self) -> usize {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
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

    fn is_suspended(&self) -> bool {
        unsafe { phonic_js_suspended() != 0 }
    }

    fn is_running(&self) -> bool {
        self.is_running
    }

    fn pause(&mut self) {
        self.is_running = false;
        self.callback_send.send(CallbackMessage::Pause).unwrap();
    }

    fn resume(&mut self) {
        self.callback_send.send(CallbackMessage::Resume).unwrap();
        self.is_running = true;
    }

    fn play(&mut self, source: Box<dyn Source>) {
        // ensure source has our sample rate and channel layout
        assert_eq!(source.channel_count(), self.channel_count());
        assert_eq!(source.sample_rate(), self.sample_rate());
        // send message to activate it in the writer
        self.callback_send
            .send(CallbackMessage::PlaySource(source))
            .unwrap();
        // auto-start with the first set source
        if !self.is_running {
            self.resume();
        }
    }

    fn stop(&mut self) {
        self.is_running = false;
        self.callback_send
            .send(CallbackMessage::PlaySource(Box::new(EmptySource)))
            .unwrap();
    }

    fn close(&mut self) {
        self.stop();
    }
}

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

// -------------------------------------------------------------------------------------------------

static mut WEBAUDIO_CONTEXT: *mut WebContext = std::ptr::null_mut();

extern "C" fn phonic_pull(num_frames: ffi::c_int) -> *const f32 {
    unsafe {
        if WEBAUDIO_CONTEXT.is_null() {
            return std::ptr::null();
        }
        let state = &mut *WEBAUDIO_CONTEXT;

        let num_frames = num_frames as usize;
        let output_samples = num_frames * state.num_channels;

        if state.buffer.len() < output_samples {
            state.buffer.resize(output_samples, 0.0);
        }
        let output = &mut state.buffer[..output_samples];

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
                    state.smoothed_volume.set_target(volume);
                }
            }
        }

        // Write out as many samples as possible from the audio source to the output buffer.
        let samples_written = match state.state {
            CallbackState::Playing => {
                let time = SourceTime {
                    pos_in_frames: state.playback_pos.load(Ordering::Relaxed)
                        / state.source.channel_count().max(1) as u64,
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
        apply_smoothed_gain(&mut output[..samples_written], &mut state.smoothed_volume);

        // Mute remaining samples, if any.
        clear_buffer(&mut output[samples_written..]);

        // Move playback pos
        state
            .playback_pos
            .fetch_add(output_samples as u64, Ordering::Relaxed);

        output.as_ptr()
    }
}

// -------------------------------------------------------------------------------------------------

struct WebContext {
    callback_recv: Receiver<CallbackMessage>,
    source: Box<dyn Source>,
    state: CallbackState,
    playback_pos: Arc<AtomicU64>,
    playback_pos_instant: Instant,
    smoothed_volume: ExponentialSmoothedValue,
    buffer: Vec<f32>,
    num_channels: usize,
}

unsafe impl Send for WebContext {}

// -------------------------------------------------------------------------------------------------

/// Stores a boxed WebContext as raw ptr and ensures the box is dropped correctly.
/// Also shuts down audio before the context gets dropped.
#[derive(Debug)]
struct WebContextRef {
    context_ptr: *mut WebContext,
}

impl Drop for WebContextRef {
    fn drop(&mut self) {
        unsafe {
            phonic_js_shutdown();
            if !self.context_ptr.is_null() {
                drop(Box::from_raw(self.context_ptr));
                WEBAUDIO_CONTEXT = std::ptr::null_mut();
            }
        }
    }
}

unsafe impl Send for WebContextRef {}
unsafe impl Sync for WebContextRef {}
