use std::sync::{mpsc::SyncSender, Arc};

use crate::{
    source::{
        amplified::AmplifiedSource, file::preloaded::PreloadedFileSource,
        mapped::ChannelMappedSource, panned::PannedSource, Source, SourceTime,
    },
    utils::{
        ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
        buffer::{scale_buffer, InterleavedBufferMut},
        dsp::modulation::{
            KeytrackingModulationSource, LfoModulationSource, ModulationMatrix, ModulationSlot,
            VelocityModulationSource, MAX_MODULATION_BLOCK_SIZE,
        },
        speed_from_note,
    },
    FileSource, NotePlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
};

use super::granular::{GrainPool, GranularParameterModulation, GranularParameters};

// -------------------------------------------------------------------------------------------------

/// Wrapped sampler voice types
type SamplerVoiceAmplifiedSource = AmplifiedSource<ChannelMappedSource<PreloadedFileSource>>;
type SamplerVoicePannedSource = PannedSource<SamplerVoiceAmplifiedSource>;
type SamplerVoiceSource = SamplerVoicePannedSource;

const GRAIN_POOL_SIZE: usize = 64;

// -------------------------------------------------------------------------------------------------

pub struct SamplerVoice {
    note_id: Option<NotePlaybackId>,
    source: SamplerVoiceSource,
    envelope: AhdsrEnvelope,
    release_start_frame: Option<u64>,
    grain_pool_started: bool,
    grain_pool: Option<Box<GrainPool<GRAIN_POOL_SIZE>>>,
    modulation_matrix: ModulationMatrix,
    modulated_size: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_density: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_variation: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_spray: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_pan_spread: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_position: [f32; MAX_MODULATION_BLOCK_SIZE],
    modulated_speed: [f32; MAX_MODULATION_BLOCK_SIZE],
    sample_rate: u32,
}

impl SamplerVoice {
    pub fn new(file_source: PreloadedFileSource, channel_count: usize, sample_rate: u32) -> Self {
        let note_id = None;

        // Create wrapped voice source
        let source = {
            // Wrap in ChannelMappedSource to match sampler's channel layout
            let channel_mapped = ChannelMappedSource::new(file_source, channel_count);
            // Wrap in AmplifiedSource for volume control
            let amplified = AmplifiedSource::new(channel_mapped, 1.0);
            // Wrap in PannedSource for panning control
            PannedSource::new(amplified, 0.0)
        };

        // Create envelope state for this voice
        let envelope = AhdsrEnvelope::new();
        let release_start_frame = None;

        // Initialize grain pool
        let grain_pool_started = false;
        let grain_pool = None;

        // Initialize modulation matrix (empty for now, will be configured later)
        let modulation_matrix = ModulationMatrix::new();
        let modulated_size = [1.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_density = [1.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_variation = [0.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_spray = [0.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_pan_spread = [0.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_position = [0.0; MAX_MODULATION_BLOCK_SIZE];
        let modulated_speed = [0.0; MAX_MODULATION_BLOCK_SIZE];

        Self {
            note_id,
            source,
            envelope,
            release_start_frame,
            grain_pool_started,
            grain_pool,
            modulation_matrix,
            modulated_size,
            modulated_density,
            modulated_variation,
            modulated_spray,
            modulated_pan_spread,
            modulated_position,
            modulated_speed,
            sample_rate,
        }
    }

    #[inline]
    /// This voice's note playback id. None, when stopped.
    pub fn note_id(&self) -> Option<NotePlaybackId> {
        self.note_id
    }

    #[inline]
    /// Is this voice currently playing something?
    pub fn is_active(&self) -> bool {
        self.note_id.is_some()
    }

    #[inline]
    /// Sample frame time when voice started its release mode.
    pub fn in_release_stage(&self) -> bool {
        self.envelope.stage() == AhdsrStage::Release
    }

    #[inline]
    /// Sample frame time when voice started its release mode.
    pub fn release_start_frame(&self) -> Option<u64> {
        self.release_start_frame
    }

    /// Set or update our file source's playback status channel.
    pub fn set_playback_status_sender(&mut self, sender: Option<SyncSender<PlaybackStatusEvent>>) {
        self.file_source_mut().set_playback_status_sender(sender);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn start(
        &mut self,
        note_id: NotePlaybackId,
        note: u8,
        volume: f32,
        panning: f32,
        envelope_parameters: &Option<AhdsrParameters>,
        context: Option<PlaybackStatusContext>,
    ) {
        // Reset a probably recycled file source
        self.reset();
        // Set initial speed, volume and pan
        let speed = speed_from_note(note);
        self.file_source_mut().set_speed(speed, None);
        self.file_source_mut().set_playback_status_context(context);
        self.amplified_source_mut().set_volume(volume);
        self.panned_source_mut().set_panning(panning);

        // Start granular playback
        if let Some(grain_pool) = &mut self.grain_pool {
            self.grain_pool_started = true;
            grain_pool.start(speed, volume, panning);
        }

        // Start envelope
        if let Some(envelope_parameters) = envelope_parameters {
            self.envelope.note_on(envelope_parameters, 1.0);
        }

        // Initialize the matrix (reset sample rate, trigger envelopes)
        self.modulation_matrix.reset(self.sample_rate);
        self.modulation_matrix.note_on(1.0); // Trigger envelopes with full volume

        self.note_id = Some(note_id);
    }

    /// Stop the voice and start fadeouts.
    pub fn stop(
        &mut self,
        envelope_parameters: &Option<AhdsrParameters>,
        current_sample_frame: u64,
    ) {
        if self.is_active() {
            self.release_start_frame = Some(current_sample_frame);

            // Trigger release phase for modulation envelopes
            self.modulation_matrix.note_off();

            // Trigger release phase for sample playback
            if let Some(envelope_parameters) = envelope_parameters {
                self.envelope.note_off(envelope_parameters);
            } else {
                self.file_source_mut().stop();
                if let Some(grain_pool) = &mut self.grain_pool {
                    grain_pool.stop();
                }
            }
        }
    }

    /// Stop & reset the voice to finish actual and prepare new playback.
    pub fn reset(&mut self) {
        if self.is_active() {
            // reset source
            self.file_source_mut().reset();
            self.file_source_mut().set_playback_status_context(None);
            self.note_id = None;
            // reset granular state
            if let Some(grain_pool) = &mut self.grain_pool {
                grain_pool.reset();
            }
        }
        // reset release start time
        self.release_start_frame = None;
    }

    /// Set a new playback speed value with optional glide.
    pub fn set_speed(&mut self, speed: f64, glide: Option<f32>) {
        self.file_source_mut().set_speed(speed, glide);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_speed(speed);
        }
    }

    /// Set a new volume value.
    pub fn set_volume(&mut self, volume: f32) {
        self.amplified_source_mut().set_volume(volume);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_volume(volume);
        }
    }

    /// Set a new panning value.
    pub fn set_panning(&mut self, panning: f32) {
        self.panned_source_mut().set_panning(panning);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_panning(panning);
        }
    }

    /// Initialize granular playback for this voice at the given sample rate.
    pub fn enable_granular_playback(&mut self, sample_rate: u32, sample_buffer: Arc<Box<[f32]>>) {
        assert!(
            !sample_buffer.is_empty(),
            "Expecting a non empty mono sample buffer here - resampled!"
        );

        // Prepare file buffer for the grain pool
        let file_buffer = self.file_source().file_buffer();
        let sample_loop_range = file_buffer.loop_range().map(|range| {
            let len = file_buffer.buffer().len() as f32;
            let start = range.start as f32 / len;
            let end = range.end as f32 / len;
            (start, end)
        });

        // Create grain pool
        self.grain_pool = Some(Box::new(GrainPool::new(
            sample_rate,
            sample_buffer,
            sample_loop_range,
        )));

        // Pre-allocate modulation matrix slots (avoid allocating them later in the audio thread)
        let modulation_matrix = &mut self.modulation_matrix;

        while modulation_matrix.lfo_slots.len() < 2 {
            let source = LfoModulationSource::new(self.sample_rate, 1.0, Default::default());
            let slot = ModulationSlot::new(source);
            modulation_matrix.add_lfo_slot(slot);
        }
        if modulation_matrix.velocity_slot.is_none() {
            let source = VelocityModulationSource::new(1.0);
            let slot = ModulationSlot::new(source);
            modulation_matrix.set_velocity_slot(slot);
        }
        if modulation_matrix.keytracking_slot.is_none() {
            let source = KeytrackingModulationSource::new(60.0);
            let slot = ModulationSlot::new(source);
            modulation_matrix.set_keytracking_slot(slot);
        }
    }

    /// Mut access to the voice modulation matrix.
    #[inline]
    pub fn modulation_matrix(&mut self) -> &mut ModulationMatrix {
        &mut self.modulation_matrix
    }

    /// Write source and apply envelope, if set.
    /// If granular_parameters is provided, renders using granular synthesis instead of continuous playback.
    pub fn process(
        &mut self,
        output: &mut [f32],
        channel_count: usize,
        envelope_parameters: &Option<AhdsrParameters>,
        granular_parameters: &Option<GranularParameters>,
        time: &SourceTime,
    ) -> usize {
        debug_assert!(self.is_active(), "Only active voices need to process");

        let written = if let Some(granular_parameters) =
            self.grain_pool.as_ref().and(granular_parameters.as_ref())
        {
            // Process in chunks of MAX_MODULATION_BLOCK_SIZE
            for chunk in output.chunks_mut(MAX_MODULATION_BLOCK_SIZE * channel_count) {
                let chunk_frame_count = chunk.len() / channel_count;

                // Process modulation for this chunk
                self.process_modulation(chunk_frame_count);

                // Process chunk with modulation
                self.grain_pool.as_mut().unwrap().process(
                    chunk,
                    channel_count,
                    granular_parameters,
                    GranularParameterModulation {
                        size: &self.modulated_size[..chunk_frame_count],
                        density: &self.modulated_density[..chunk_frame_count],
                        variation: &self.modulated_variation[..chunk_frame_count],
                        spray: &self.modulated_spray[..chunk_frame_count],
                        pan_spread: &self.modulated_pan_spread[..chunk_frame_count],
                        position: &self.modulated_position[..chunk_frame_count],
                        speed: &self.modulated_speed[..chunk_frame_count],
                    },
                );
            }

            output.len()
        } else {
            // Continuous playback mode
            self.source.write(output, time)
        };

        // Get current t recent odulation value for position parameters
        let pos_mod = if self.modulation_matrix.output_size() > 0 {
            self.modulation_matrix.modulation_output_at(
                super::Sampler::GRAIN_POSITION.id(),
                self.modulation_matrix.output_size() - 1,
            )
        } else {
            0.0
        };

        // Send playhead as playback position
        if let Some(grain_pool_playhead) = self
            .grain_pool
            .as_ref()
            .zip(granular_parameters.as_ref())
            .map(|(pool, parameters)| pool.playback_position(parameters, pos_mod))
        {
            let sample_buffer = self.file_source().file_buffer();
            let is_start_event = self.grain_pool_started;
            self.grain_pool_started = false;
            self.file_source_mut()
                .file_source_impl_mut()
                .send_playback_position_status(
                    time,
                    is_start_event,
                    (grain_pool_playhead * sample_buffer.buffer().len() as f32) as u64,
                    sample_buffer.channel_count(),
                    sample_buffer.sample_rate(),
                );
        }

        // Apply envelope to the voice output
        if let Some(envelope_parameters) = envelope_parameters {
            let mut output = &mut output[..written];
            if matches!(
                self.envelope.stage(),
                AhdsrStage::Sustain | AhdsrStage::Idle
            ) {
                // no need to run the envelope per frame in sustain or idle state
                scale_buffer(output, self.envelope.output());
            } else {
                for frame in output.frames_mut(channel_count) {
                    let envelope_value = self.envelope.run(envelope_parameters);
                    for sample in frame {
                        *sample *= envelope_value;
                    }
                }
            }
        }

        // Check if voice finished playback or envelope finished
        if self.source.is_exhausted()
            || self.grain_pool.as_ref().is_some_and(|s| s.is_exhausted())
            || (envelope_parameters.is_some() && self.envelope.stage() == AhdsrStage::Idle)
        {
            // Reset voice playback
            self.reset();

            // Send grain playback stop
            if let Some(grain_pool_exhausted) = self.grain_pool.as_ref().map(|p| p.is_exhausted()) {
                self.file_source_mut()
                    .file_source_impl_mut()
                    .send_playback_stopped_status(grain_pool_exhausted);
            }
        }

        written
    }

    #[inline]
    pub(crate) fn panned_source_mut(&mut self) -> &mut SamplerVoicePannedSource {
        &mut self.source
    }

    #[inline]
    pub(crate) fn amplified_source_mut(&mut self) -> &mut SamplerVoiceAmplifiedSource {
        self.source.input_source_mut()
    }

    #[inline]
    pub(crate) fn file_source(&self) -> &PreloadedFileSource {
        self.source.input_source().input_source().input_source()
    }
    #[inline]
    pub(crate) fn file_source_mut(&mut self) -> &mut PreloadedFileSource {
        self.source
            .input_source_mut()
            .input_source_mut()
            .input_source_mut()
    }

    /// Process modulation block and fill modulation buffers.
    /// Called once per chunk of up to MODULATION_BLOCK_SIZE samples.
    ///
    /// # Arguments
    /// * `base_params` - Base granular parameters to modulate
    /// * `chunk_frames` - Number of frames to process (up to MODULATION_BLOCK_SIZE)
    fn process_modulation(&mut self, chunk_frames: usize) {
        use super::Sampler;

        debug_assert!(
            chunk_frames <= MAX_MODULATION_BLOCK_SIZE,
            "Chunk frames exceeds maximum block size"
        );

        // Process modulation sources for this chunk
        self.modulation_matrix.process(chunk_frames);

        // Fill modulation buffers for the chunk
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_SIZE.id(),
            &mut self.modulated_size[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_DENSITY.id(),
            &mut self.modulated_density[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_VARIATION.id(),
            &mut self.modulated_variation[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_SPRAY.id(),
            &mut self.modulated_spray[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_PAN_SPREAD.id(),
            &mut self.modulated_pan_spread[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_POSITION.id(),
            &mut self.modulated_position[..chunk_frames],
        );
        self.modulation_matrix.modulation_output(
            Sampler::GRAIN_SPEED.id(),
            &mut self.modulated_speed[..chunk_frames],
        );
    }
}
