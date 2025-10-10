use std::{
    fs::File,
    io::BufWriter,
    path::Path,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::{
    error::Error,
    output::OutputDevice,
    source::{empty::EmptySource, Source, SourceTime},
    utils::smoothing::{apply_smoothed_gain, ExponentialSmoothedValue, SmoothedValue},
};

use hound::{SampleFormat, WavSpec, WavWriter};

// -------------------------------------------------------------------------------------------------

const DEFAULT_SAMPLE_RATE: u32 = 44100;
const DEFAULT_CHANNEL_COUNT: usize = 2;
const DEFAULT_DURATION: Duration = Duration::from_secs(u64::MAX);

const BUFFER_SIZE_FRAMES: usize = 1024;

// -------------------------------------------------------------------------------------------------

/// Audio output device, which writes audio into a wav file instead of playing it back.
///
/// NOTE: Unlike the other output devices, the wav writer device is initially paused, so it
/// must be resumed (or started via the player) manually after everything you want to write
/// via the player got set up.
pub struct WavOutput {
    stream: Arc<Mutex<WavStream>>,
}

impl WavOutput {
    /// Open a wav output device to write at the given file using default specs and an
    /// endless duration.
    pub fn open<P: AsRef<Path>>(file_path: P) -> Result<Self, Error> {
        Self::open_with_specs(
            file_path,
            DEFAULT_SAMPLE_RATE,
            DEFAULT_CHANNEL_COUNT,
            DEFAULT_DURATION,
        )
    }

    /// Create a new wav output device with the given parameters.
    ///
    /// * `file_path`: Target file path. Should end with ".wav" extension.
    /// * `sample_rate`: Player and wav file's target sample rate.
    /// * `channel_count`: Player and wav file's channel layout.
    /// * `duration`: Max length of written content. When the player's source no longer
    ///   produces any output (e.g. is stopped), the wav file will be closed automatically,
    ///   so the duration also can be endless to stop automatically.
    ///
    /// Wav files contents are always saved as 32bit floats.
    pub fn open_with_specs<P: AsRef<Path>>(
        file_path: P,
        sample_rate: u32,
        channel_count: usize,
        duration: Duration,
    ) -> Result<Self, Error> {
        let spec = WavSpec {
            channels: channel_count as u16,
            sample_rate,
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
        };

        let writer = WavWriter::create(file_path, spec)
            .map_err(|e| Error::OutputDeviceError(Box::new(e)))?;

        let stream = Arc::new(Mutex::new(WavStream {
            writer: Some(writer),
            channel_count,
            sample_rate,
            source: Box::new(EmptySource),
            smoothed_volume: ExponentialSmoothedValue::new(1.0, spec.sample_rate),
            buffer: vec![0.0; BUFFER_SIZE_FRAMES * spec.channels as usize],
            started: false,
            finished: false,
            playback_pos: 0,
            duration,
        }));

        // Start the stream in a new detached thread
        thread::spawn({
            let stream = Arc::clone(&stream);
            move || {
                loop {
                    // process the next audio slice
                    {
                        let mut stream = stream.lock().unwrap();
                        if let Err(err) = stream.process() {
                            panic!("Error processing WAV output: {err}");
                        }
                        // Stop write loop when duration elapsed
                        if stream.finished {
                            stream.started = false;
                            break;
                        }
                    }
                    // sleep for a short time to avoid busy waiting
                    thread::sleep(Duration::from_millis(1));
                }

                // Finalize the WAV file when done
                if let Ok(mut stream) = stream.lock() {
                    if let Some(writer) = stream.writer.take() {
                        if let Err(e) = writer.finalize() {
                            log::error!("Failed to finalize WAV file: {e}");
                        }
                    }
                }
            }
        });

        Ok(Self { stream })
    }
}

impl OutputDevice for WavOutput {
    fn channel_count(&self) -> usize {
        let inner = self.stream.lock().unwrap();
        inner.channel_count
    }

    fn sample_rate(&self) -> u32 {
        let inner = self.stream.lock().unwrap();
        inner.sample_rate
    }

    fn sample_position(&self) -> u64 {
        let inner = self.stream.lock().unwrap();
        inner.playback_pos
    }

    fn volume(&self) -> f32 {
        let inner = self.stream.lock().unwrap();
        inner.smoothed_volume.target()
    }

    fn set_volume(&mut self, volume: f32) {
        let mut inner = self.stream.lock().unwrap();
        inner.smoothed_volume.set_target(volume);
    }

    fn is_suspended(&self) -> bool {
        false
    }

    fn is_running(&self) -> bool {
        let inner = self.stream.lock().unwrap();
        inner.started
    }

    fn pause(&mut self) {
        let mut inner = self.stream.lock().unwrap();
        inner.started = false;
    }

    fn resume(&mut self) {
        let mut inner = self.stream.lock().unwrap();
        inner.started = true;
    }

    fn play(&mut self, source: Box<dyn Source>) {
        let mut inner = self.stream.lock().unwrap();
        // ensure source has our sample rate and channel layout
        assert_eq!(source.channel_count(), inner.channel_count);
        assert_eq!(source.sample_rate(), inner.sample_rate);
        inner.source = source;
    }

    fn stop(&mut self) {
        let mut inner = self.stream.lock().unwrap();
        inner.source = Box::new(EmptySource);
    }

    fn close(&mut self) {
        let mut inner = self.stream.lock().unwrap();
        inner.finished = true;
    }
}

// -------------------------------------------------------------------------------------------------

struct WavStream {
    writer: Option<WavWriter<BufWriter<File>>>,
    channel_count: usize,
    sample_rate: u32,
    source: Box<dyn Source>,
    smoothed_volume: ExponentialSmoothedValue,
    buffer: Vec<f32>,
    started: bool,
    finished: bool,
    playback_pos: u64,
    duration: Duration,
}

impl WavStream {
    fn process(&mut self) -> Result<(), Error> {
        // Do nothing when we didn't started yet
        if !self.started || self.finished {
            return Ok(());
        }
        // Calculate source time
        let time = SourceTime {
            pos_in_frames: self.playback_pos / self.channel_count as u64,
            pos_instant: Instant::now(),
        };

        // Stop running when we've exceeded the duration
        if Duration::from_secs(time.pos_in_frames / self.sample_rate as u64) >= self.duration {
            self.finished = true;
            return Ok(());
        }

        // Write out as many samples as possible from the audio source to the buffer.
        let written = self.source.write(&mut self.buffer, &time);

        // Stop writing when no more output is produced
        if written == 0 {
            self.finished = true;
            return Ok(());
        }

        // Apply the global volume level
        apply_smoothed_gain(&mut self.buffer[..written], &mut self.smoothed_volume);

        // Write to WAV file
        if let Some(ref mut writer) = self.writer {
            for sample in &self.buffer[..written] {
                if let Err(err) = writer.write_sample(*sample) {
                    panic!("Failed to write sample to WAV file: {err}");
                }
            }
        }

        self.playback_pos += self.buffer.len() as u64;
        Ok(())
    }
}

impl Drop for WavStream {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            if let Err(err) = writer.finalize() {
                panic!("Failed to finalize WAV file: {err}");
            }
        }
    }
}
