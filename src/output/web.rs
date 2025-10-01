#[cfg(feature = "assert-allocs")]
use assert_no_alloc::*;

use std::{
    ffi,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};

use std::sync::mpsc::{sync_channel, Receiver, SyncSender};

use emscripten_rs_sys::{
    emscripten_audio_context_state, emscripten_audio_node_connect, emscripten_create_audio_context,
    emscripten_create_wasm_audio_worklet_node,
    emscripten_create_wasm_audio_worklet_processor_async, emscripten_destroy_audio_context,
    emscripten_main_runtime_thread_id, emscripten_resume_audio_context_sync,
    emscripten_set_click_callback_on_thread, emscripten_start_wasm_audio_worklet_thread_async, js,
    AudioParamFrame, AudioSampleFrame, EmscriptenAudioWorkletNodeCreateOptions,
    EmscriptenMouseEvent, WebAudioWorkletProcessorCreateOptions, EMSCRIPTEN_WEBAUDIO_T,
};

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

/// WebAudio backend to use.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum WebBackend {
    /// Use `ScriptProcessorNode`. This is deprecated but has better browser compatibility.
    ScriptProcessorNode,
    #[default]
    /// Use Audio Worklet API. This is the preferred, modern way and requires a secure context (HTTPS).
    AudioWorklet,
}

// -------------------------------------------------------------------------------------------------

const PREFERRED_SAMPLE_RATE: i32 = 44100;
const PREFERRED_CHANNELS: i32 = 2;
const PREFERRED_BUFFER_SIZE: i32 = if cfg!(debug_assertions) {
    2048 * PREFERRED_CHANNELS
} else {
    1024 * PREFERRED_CHANNELS
};

// -------------------------------------------------------------------------------------------------

// JS impl of the WebAudio backend, based on the https://github.com/floooh/sokol audio impl.
//
// This is currently using a ScriptProcessorNode callback to feed the sample data into WebAudio.
// ScriptProcessorNode has been deprecated for a while because it is running from the main thread,
// with the default initialization parameters it works 'pretty well' though.
//
// The magic `js!` embedding is done via https://docs.rs/crate/emscripten_rs_sys and only works in
// rust nightly builds with `asm_experimental_arch` and `macro_metavar_expr_concat` features enabled.

#[cfg(not(target_os = "emscripten"))]
compile_error!("The 'web-output' feature currently is implemented for emscripten only.");

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

// return sample rate of the audio context. Used for audio worklets only.
js! {
    fn get_audio_context_sample_rate(context: EMSCRIPTEN_WEBAUDIO_T) -> ffi::c_double,
    {
        var AudioContext = window.AudioContext || window.webkitAudioContext;
        var ctx = new AudioContext();
        var sr = ctx.sampleRate;
        ctx.close();
        return sr;
    };
}

// -------------------------------------------------------------------------------------------------

/// Audio [`OutputDevice`] impl using WebAudio with a ScriptProcessorNode or AudioWorklet.
///
/// Should be primally used as audio output for emscripten builds, because cpal's emscripten
/// impls are broken and no longer maintained.
#[derive(Debug)]
pub struct WebOutput {
    volume: f32,
    is_running: bool,
    playback_pos: Arc<AtomicU64>,
    callback_sender: SyncSender<CallbackMessage>,
    #[allow(dead_code)]
    context_ref: Arc<WebContextRef>,
    channel_count: usize,
    sample_rate: u32,
    backend: WebBackend,
    webaudio_context_handle: EMSCRIPTEN_WEBAUDIO_T,
}

impl WebOutput {
    pub fn open() -> Result<Self, Error> {
        Self::with_backend(WebBackend::default())
    }

    pub fn with_backend(backend: WebBackend) -> Result<Self, Error> {
        let playback_pos = Arc::new(AtomicU64::new(0));
        let (callback_sender, callback_receiver) = sync_channel(16);

        let (sample_rate, channel_count, _buffer_frames, context_ptr, webaudio_context_handle) =
            match backend {
                WebBackend::ScriptProcessorNode => {
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

                    let channel_count = PREFERRED_CHANNELS as usize;
                    let sample_rate = unsafe { phonic_js_sample_rate() } as u32;
                    let buffer_frames = unsafe { phonic_js_buffer_frames() } as usize;

                    let context = Box::new(WebContext {
                        callback_receiver,
                        source: Box::new(EmptySource),
                        playback_pos: Arc::clone(&playback_pos),
                        playback_pos_instant: Instant::now(),
                        state: CallbackState::Paused,
                        smoothed_volume: ExponentialSmoothedValue::new(1.0, sample_rate),
                        buffer: vec![0.0; buffer_frames * channel_count],
                        num_channels: channel_count,
                    });
                    let context_ptr = Box::into_raw(context);

                    let webaudio_context_handle = 0;

                    (
                        sample_rate,
                        channel_count,
                        buffer_frames,
                        context_ptr,
                        webaudio_context_handle,
                    )
                }
                WebBackend::AudioWorklet => {
                    println!("phonic: Creating audio context...");

                    let webaudio_context_handle =
                        unsafe { emscripten_create_audio_context(std::ptr::null()) };
                    if webaudio_context_handle == 0 {
                        return Err(Error::OutputDeviceError(
                            "Failed to create WebAudio context".into(),
                        ));
                    }

                    // Resume context on user interaction
                    extern "C" fn on_user_interaction(
                        _event_type: ffi::c_int,
                        _mouse_event: *const EmscriptenMouseEvent,
                        user_data: *mut ffi::c_void,
                    ) -> bool {
                        let context_handle = user_data as EMSCRIPTEN_WEBAUDIO_T;
                        if context_handle != 0 {
                            unsafe {
                                emscripten_resume_audio_context_sync(context_handle);
                            }
                        }
                        false
                    }

                    let body_selector = ffi::CString::new("body").unwrap();
                    unsafe {
                        emscripten_set_click_callback_on_thread(
                            body_selector.as_ptr(),
                            webaudio_context_handle as *mut ffi::c_void,
                            false,
                            Some(on_user_interaction),
                            emscripten_main_runtime_thread_id(),
                        );
                    }

                    let sample_rate =
                        unsafe { get_audio_context_sample_rate(webaudio_context_handle) } as u32;
                    let channel_count = PREFERRED_CHANNELS as usize;
                    let buffer_frames = 128; // AudioWorklet is fixed to 128 frames

                    println!("phonic: Audio worklet initialized with sample_rate: {sample_rate}");

                    let context = Box::new(WebContext {
                        callback_receiver,
                        source: Box::new(EmptySource),
                        playback_pos: Arc::clone(&playback_pos),
                        playback_pos_instant: Instant::now(),
                        state: CallbackState::Paused,
                        smoothed_volume: ExponentialSmoothedValue::new(1.0, sample_rate),
                        buffer: vec![0.0; buffer_frames * channel_count],
                        num_channels: channel_count,
                    });
                    let context_ptr = Box::into_raw(context);

                    println!("phonic: Starting audio worklet thread...");

                    const STACK_SIZE: usize = 1024 * 1024 * 2;
                    let mut stack = unsafe {
                        // create 16 byte aligned vector as stack
                        let n_units = (STACK_SIZE / std::mem::size_of::<u128>()) + 1;
                        let mut aligned: Vec<u128> = Vec::with_capacity(n_units);

                        let ptr = aligned.as_mut_ptr();
                        let len_units = aligned.len();
                        let cap_units = aligned.capacity();

                        std::mem::forget(aligned);

                        Vec::from_raw_parts(
                            ptr as *mut u8,
                            len_units * std::mem::size_of::<u128>(),
                            cap_units * std::mem::size_of::<u128>(),
                        )
                    };

                    unsafe {
                        emscripten_start_wasm_audio_worklet_thread_async(
                            webaudio_context_handle,
                            stack.as_mut_ptr() as *mut ffi::c_void,
                            STACK_SIZE as u32,
                            Some(worklet_audio_thread_initialized),
                            context_ptr as *mut ffi::c_void,
                        );
                    }

                    // Leak the stack: needed for the lifetime of the worklet thread
                    std::mem::forget(stack);

                    (
                        sample_rate,
                        channel_count,
                        buffer_frames,
                        context_ptr,
                        webaudio_context_handle,
                    )
                }
            };

        let volume = 1.0;
        let is_running = false;

        unsafe {
            WEBAUDIO_CONTEXT = context_ptr;
        }

        let context_ref = Arc::new(WebContextRef { context_ptr });

        Ok(Self {
            volume,
            is_running,
            playback_pos,
            callback_sender,
            context_ref,
            channel_count,
            sample_rate,
            backend,
            webaudio_context_handle,
        })
    }

    fn send_to_callback(&self, msg: CallbackMessage) {
        if let Err(err) = self.callback_sender.send(msg) {
            log::error!("Failed to send callback message: {err}");
        }
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
        self.send_to_callback(CallbackMessage::SetVolume(volume));
    }

    fn is_suspended(&self) -> bool {
        match self.backend {
            WebBackend::ScriptProcessorNode => unsafe { phonic_js_suspended() != 0 },
            WebBackend::AudioWorklet => {
                const AUDIO_CONTEXT_STATE_SUSPENDED: ffi::c_int = 1;
                unsafe {
                    emscripten_audio_context_state(self.webaudio_context_handle)
                        == AUDIO_CONTEXT_STATE_SUSPENDED
                }
            }
        }
    }

    fn is_running(&self) -> bool {
        self.is_running
    }

    fn pause(&mut self) {
        self.is_running = false;
        self.send_to_callback(CallbackMessage::Pause);
    }

    fn resume(&mut self) {
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
        self.is_running = false;
        self.send_to_callback(CallbackMessage::PlaySource(Box::new(EmptySource)));
    }

    fn close(&mut self) {
        self.stop();
    }
}

impl Drop for WebOutput {
    fn drop(&mut self) {
        if self.backend == WebBackend::AudioWorklet && self.webaudio_context_handle != 0 {
            unsafe {
                emscripten_destroy_audio_context(self.webaudio_context_handle);
                self.webaudio_context_handle = 0;
            }
        }
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

// Audio worklet backend impl using Emscripten's custom worklet impls. Using plain JS is hard to
// realize as the processors can only be created within a AudioWorkletGlobalScope.

extern "C" fn worklet_audio_thread_initialized(
    context_handle: EMSCRIPTEN_WEBAUDIO_T,
    success: bool,
    user_data: *mut ffi::c_void,
) {
    if !success {
        println!("Audio worklet thread FAILED to initialize");
        return;
    }

    println!("phonic: Creating audio worklet processor...");

    let processor_name = ffi::CString::new("phonic-processor").unwrap();
    let opts = WebAudioWorkletProcessorCreateOptions {
        name: processor_name.as_ptr(),
        numAudioParams: 0,
        audioParamDescriptors: std::ptr::null(),
    };

    unsafe {
        emscripten_create_wasm_audio_worklet_processor_async(
            context_handle,
            &opts,
            Some(worklet_processor_created),
            user_data,
        );
        std::mem::forget(processor_name); // Leak CString
    }
}

extern "C" fn worklet_processor_created(
    context_handle: EMSCRIPTEN_WEBAUDIO_T,
    success: bool,
    user_data: *mut ffi::c_void,
) {
    if !success {
        println!("phonic: Creating audio worklet processor FAILED");
        return;
    }

    println!("phonic: Creating audio worklet node...");

    let mut output_channel_counts = [PREFERRED_CHANNELS];
    let node_opts = EmscriptenAudioWorkletNodeCreateOptions {
        numberOfInputs: 0,
        numberOfOutputs: 1,
        outputChannelCounts: output_channel_counts.as_mut_ptr(),
    };

    let processor_name = ffi::CString::new("phonic-processor").unwrap();
    let node = unsafe {
        emscripten_create_wasm_audio_worklet_node(
            context_handle,
            processor_name.as_ptr(),
            &node_opts,
            Some(worklet_process_audio),
            user_data,
        )
    };
    std::mem::forget(processor_name); // leak!

    unsafe {
        emscripten_audio_node_connect(node, context_handle, 0, 0);
    }

    println!("phonic: Audio worklet node is up and running");
}

extern "C" fn worklet_process_audio(
    _num_inputs: ffi::c_int,
    _inputs: *const AudioSampleFrame,
    num_outputs: ffi::c_int,
    outputs: *mut AudioSampleFrame,
    _num_params: ffi::c_int,
    _params: *const AudioParamFrame,
    user_data: *mut ffi::c_void,
) -> bool {
    if num_outputs == 0 || user_data.is_null() {
        return true;
    }

    // set context, first time we got called
    unsafe {
        if WEBAUDIO_CONTEXT.is_null() {
            WEBAUDIO_CONTEXT = user_data as *mut WebContext;
        }
    }

    // pull frames and copy to output
    let output_frame = unsafe { &mut *outputs };
    let num_frames = output_frame.samplesPerChannel as usize;
    let num_channels = output_frame.numberOfChannels as usize;

    let buffer_ptr = phonic_pull(num_frames as ffi::c_int);

    if !buffer_ptr.is_null() {
        let output_len = num_frames * num_channels;
        let buffer_slice = unsafe { std::slice::from_raw_parts(buffer_ptr, output_len) };
        let output_slice = unsafe { std::slice::from_raw_parts_mut(output_frame.data, output_len) };
        output_slice.copy_from_slice(buffer_slice);
    }

    true // keep processing
}

// -------------------------------------------------------------------------------------------------

static mut WEBAUDIO_CONTEXT: *mut WebContext = std::ptr::null_mut();

// Audio pull functions as used by the worklet and script processor backends.

extern "C" fn phonic_pull(num_frames: ffi::c_int) -> *const f32 {
    unsafe {
        if WEBAUDIO_CONTEXT.is_null() {
            return std::ptr::null();
        }

        let state = &mut *WEBAUDIO_CONTEXT;

        let num_frames = num_frames as usize;
        let num_channels = state.num_channels;
        let output_samples = num_frames * num_channels;

        if state.buffer.len() < output_samples {
            state.buffer.resize(output_samples, 0.0);
        }
        let output = &mut state.buffer[..output_samples];

        // Process any pending data messages.
        while let Ok(msg) = state.callback_receiver.try_recv() {
            match msg {
                CallbackMessage::PlaySource(src) => {
                    log::debug!("Setting new stream source...");
                    state.source = src;
                }
                CallbackMessage::Pause => {
                    log::debug!("Pausing audio output stream...");
                    state.state = CallbackState::Paused;
                }
                CallbackMessage::Resume => {
                    log::debug!("Resuming audio output stream...");
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
                /*{
                    // Sine wave generation for testing
                    static mut SINE_PHASE: f32 = 0.0;
                    const SINE_FREQ: f32 = 100.0;
                    const SAMPLE_RATE: f32 = 48000.0;

                    let phase_increment = std::f32::consts::TAU * SINE_FREQ / SAMPLE_RATE;

                    for frame in 0..num_frames {
                        let value = unsafe { SINE_PHASE.sin() * 0.25 };
                        unsafe {
                            SINE_PHASE += phase_increment;
                            if SINE_PHASE > std::f32::consts::TAU {
                                SINE_PHASE -= std::f32::consts::TAU;
                            }
                        }
                        for channel in 0..num_channels {
                            output[frame * num_channels + channel] = value;
                        }
                    }

                    num_frames * num_channels
                }*/
                {
                    let time = SourceTime {
                        pos_in_frames: state.playback_pos.load(Ordering::Relaxed)
                            / num_channels as u64,
                        pos_instant: state.playback_pos_instant,
                    };
                    #[cfg(not(feature = "assert-allocs"))]
                    {
                        state.source.write(output, &time)
                    }
                    #[cfg(feature = "assert-allocs")]
                    {
                        assert_no_alloc(|| state.source.write(output, &time))
                    }
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
    callback_receiver: Receiver<CallbackMessage>,
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
