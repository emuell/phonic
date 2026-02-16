use std::sync::{mpsc::SyncSender, Arc};

use crate::{
    modulation::{matrix::ModulationMatrix, processor::MODULATION_PROCESSOR_BLOCK_SIZE},
    source::{
        amplified::AmplifiedSource, file::preloaded::PreloadedFileSource,
        mapped::ChannelMappedSource, panned::PannedSource, Source, SourceTime,
    },
    utils::{
        ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
        buffer::{scale_buffer, InterleavedBufferMut},
        speed_from_note,
    },
    FileSource, NotePlaybackId, PlaybackStatusContext, PlaybackStatusEvent,
};

use super::{
    granular::{GrainPool, GranularParameters},
    modulation::SamplerVoiceModulationState,
};

// -------------------------------------------------------------------------------------------------

/// Wrapped sampler voice types
type SamplerVoiceAmplifiedSource = AmplifiedSource<ChannelMappedSource<PreloadedFileSource>>;
type SamplerVoicePannedSource = PannedSource<SamplerVoiceAmplifiedSource>;
type SamplerVoiceSource = SamplerVoicePannedSource;

// Fit 100 grains with a max density of 100Hz and a max grain size of 100ms
const GRAIN_POOL_SIZE: usize = 100;

// -------------------------------------------------------------------------------------------------

pub(crate) struct SamplerVoice {
    note_id: Option<NotePlaybackId>,
    note: u8,
    note_volume: f32,
    note_panning: f32,
    source: SamplerVoiceSource,
    envelope: AhdsrEnvelope,
    release_start_frame: Option<u64>,
    grain_pool_started: bool,
    grain_pool: Option<Box<GrainPool<GRAIN_POOL_SIZE>>>,
    modulation_state: Option<Box<SamplerVoiceModulationState>>,
}

impl SamplerVoice {
    pub fn new(file_source: PreloadedFileSource, channel_count: usize, _sample_rate: u32) -> Self {
        let note_id = None;
        let note = 60; // middle C
        let note_volume = 1.0;
        let note_panning = 0.0;

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

        // Initialize modulation matrix (empty without granular playback enabled)
        let modulation_state = None;

        Self {
            note_id,
            note,
            note_volume,
            note_panning,
            source,
            envelope,
            release_start_frame,
            grain_pool_started,
            grain_pool,
            modulation_state,
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
        base_transpose: i32,
        base_finetune: i32,
        base_volume: f32,
        base_panning: f32,
        envelope_parameters: &Option<AhdsrParameters>,
        granular_parameters: &Option<GranularParameters>,
        context: Option<PlaybackStatusContext>,
    ) {
        // Reset a probably recycled file source
        self.reset();

        // Store per-note values for later recomputation
        self.note = note;
        self.note_volume = volume;
        self.note_panning = panning;

        // Compute effective speed: note speed * pitch factor from transpose + finetune
        let note_speed = speed_from_note(note);
        let pitch_factor =
            2.0_f64.powf((base_transpose as f64) / 12.0 + (base_finetune as f64) / 1200.0);
        let effective_speed = note_speed * pitch_factor;

        // Compute effective volume and panning
        let effective_volume = base_volume * volume;
        let effective_panning = (base_panning + panning).clamp(-1.0, 1.0);

        // Apply to source chain
        self.file_source_mut().set_speed(effective_speed, None);
        self.amplified_source_mut().set_volume(effective_volume);
        self.panned_source_mut().set_panning(effective_panning);

        // Start granular playback with effective values
        debug_assert!(
            self.grain_pool.is_some() == granular_parameters.is_some(),
            "Expecting valid grain parameters when granular playback is enabled",
        );
        if let Some((grain_pool, granular_parameters)) = self
            .grain_pool
            .as_deref_mut()
            .zip(granular_parameters.as_ref())
        {
            self.grain_pool_started = true;
            grain_pool.start(
                granular_parameters,
                effective_speed,
                effective_volume,
                effective_panning,
            );
        }

        // Set playback context
        self.file_source_mut().set_playback_status_context(context);

        // Initialize volume envelope
        if let Some(envelope_parameters) = envelope_parameters {
            self.envelope.note_on(envelope_parameters, 1.0); // Trigger envelopes with full volume
        }

        // Initialize modulation matrix
        if let Some(state) = &mut self.modulation_state {
            state.start(note, volume);
        }

        // Memorize note id and act as active
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

            // Trigger release phase for sample playback
            if let Some(envelope_parameters) = envelope_parameters {
                self.envelope.note_off(envelope_parameters);
            } else {
                self.file_source_mut().stop();
                if let Some(grain_pool) = &mut self.grain_pool {
                    grain_pool.stop();
                }
            }

            // Trigger release phase for modulation
            if let Some(state) = &mut self.modulation_state {
                state.stop();
            }
        }
    }

    /// Stop & reset the voice to finish actual and prepare new playback.
    pub fn reset(&mut self) {
        if self.is_active() {
            // reset source
            self.file_source_mut().reset();
            self.file_source_mut().set_playback_status_context(None);
            // note properties are left as they are: they will be overwritten in start()
            self.note_id = None;
            // reset granular state
            if let Some(grain_pool) = &mut self.grain_pool {
                grain_pool.reset();
            }
        }
        // reset release start time
        self.release_start_frame = None;
    }

    /// This is called when a SetSpeed event is applied for a specific note.
    pub fn set_speed(
        &mut self,
        speed: f64,
        glide: Option<f32>,
        base_transpose: i32,
        base_finetune: i32,
    ) {
        // Compute effective speed: note speed * pitch factor from transpose + finetune
        let pitch_factor =
            2.0_f64.powf((base_transpose as f64) / 12.0 + (base_finetune as f64) / 1200.0);
        let effective_speed = speed * pitch_factor;
        self.file_source_mut().set_speed(effective_speed, glide);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_speed(effective_speed);
        }
    }

    /// Recompute and apply the effective speed from stored note + base transpose/finetune.
    /// This is called when the sampler's base pitch changes during playback.
    pub fn set_base_pitch(&mut self, base_transpose: i32, base_finetune: i32) {
        // Clear any speed override -- transpose/finetune takes precedence
        let note_speed = speed_from_note(self.note);
        let pitch_factor =
            2.0_f64.powf((base_transpose as f64) / 12.0 + (base_finetune as f64) / 1200.0);
        let effective_speed = note_speed * pitch_factor;
        self.file_source_mut().set_speed(effective_speed, None);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_speed(effective_speed);
        }
    }

    /// Set a new per-note volume value. Composes with base volume.
    /// This is called when a SetVolume event is applied for a specific note.
    pub fn set_volume(&mut self, volume: f32, base_volume: f32) {
        self.note_volume = volume;
        let effective_volume = base_volume * volume;
        self.amplified_source_mut().set_volume(effective_volume);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_volume(effective_volume);
        }
    }

    /// Recompute and apply the effective volume from stored per-note volume + base volume.
    /// This is called when the sampler's base volume changes during playback.
    pub fn set_base_volume(&mut self, base_volume: f32) {
        let effective_volume = base_volume * self.note_volume;
        self.amplified_source_mut().set_volume(effective_volume);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_volume(effective_volume);
        }
    }

    /// Set a new per-note panning value. Composes with base panning.
    /// This is called when a SetPanning event is applied for a specific note.
    pub fn set_panning(&mut self, panning: f32, base_panning: f32) {
        self.note_panning = panning;
        let effective_panning = (base_panning + panning).clamp(-1.0, 1.0);
        self.panned_source_mut().set_panning(effective_panning);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_panning(effective_panning);
        }
    }

    /// Recompute and apply the effective panning from stored per-note panning + base panning.
    /// This is called when the sampler's base panning changes during playback.
    pub fn set_base_panning(&mut self, base_panning: f32) {
        let effective_panning = (base_panning + self.note_panning).clamp(-1.0, 1.0);
        self.panned_source_mut().set_panning(effective_panning);
        if let Some(grain_pool) = &mut self.grain_pool {
            grain_pool.set_panning(effective_panning);
        }
    }

    /// Initialize granular playback for this voice with the given sample rate.
    pub fn enable_granular_playback(
        &mut self,
        modulation_matrix: ModulationMatrix,
        sample_rate: u32,
        sample_buffer: Arc<Box<[f32]>>,
    ) {
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

        // Setup grain modulation matrix
        self.modulation_state = Some(Box::new(SamplerVoiceModulationState::new(
            modulation_matrix,
        )));
    }

    /// Access to the voice modulation matrix.
    #[inline]
    #[allow(unused)]
    pub fn modulation_matrix(&self) -> Option<&ModulationMatrix> {
        self.modulation_state.as_ref().map(|s| s.matrix())
    }

    /// Mut access to the voice modulation matrix.
    #[inline]
    pub fn modulation_matrix_mut(&mut self) -> Option<&mut ModulationMatrix> {
        self.modulation_state.as_mut().map(|s| s.matrix_mut())
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

        debug_assert!(
            self.grain_pool.is_some() == granular_parameters.is_some()
                && self.grain_pool.is_some() == self.modulation_state.is_some(),
            "Expecting grain pool, parameters and modulation to be enabled or disabled together"
        );

        let written = match (
            self.grain_pool.as_deref_mut(),
            self.modulation_state.as_deref_mut(),
            granular_parameters.as_ref(),
        ) {
            // Grain playback mode
            (Some(grain_pool), Some(modulation_state), Some(granular_parameters)) => {
                // Process in chunks of MODULATION_PROCESSOR_BLOCK_SIZE
                for chunk in output.chunks_mut(MODULATION_PROCESSOR_BLOCK_SIZE * channel_count) {
                    let chunk_frame_count = chunk.len() / channel_count;
                    // Process modulation for this chunk
                    modulation_state.process(chunk_frame_count);
                    // Process chunk with modulation
                    grain_pool.process(
                        chunk,
                        channel_count,
                        granular_parameters,
                        &modulation_state.output(chunk_frame_count),
                    );
                }
                output.len()
            }
            _ => {
                // Regular file playback mode
                self.source.write(output, time)
            }
        };

        // Get current modulation value for position parameters
        let pos_mod = if let Some(state) = &self.modulation_state {
            if state.matrix().output_size() > 0 {
                state.matrix().output_at(
                    super::Sampler::GRAIN_POSITION.id(),
                    state.matrix().output_size() - 1,
                )
            } else {
                0.0
            }
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
}
