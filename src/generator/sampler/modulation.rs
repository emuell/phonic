use four_cc::FourCC;

use crate::{
    modulation::{
        matrix::ModulationMatrix,
        processor::MODULATION_PROCESSOR_BLOCK_SIZE,
        state::{ModulationSlotType, ModulationState},
        ModulationConfig, ModulationSource, ModulationTarget,
    },
    utils::dsp::lfo::LfoWaveform,
    Error,
};

use super::{granular::GranularParameterModulation, voice::SamplerVoice};

// -------------------------------------------------------------------------------------------------

/// Modulation state for the sampler generator.
///
/// Wraps shared `ModulationState`.
#[derive(Debug)]
pub(crate) struct SamplerModulationState {
    inner: ModulationState,
}

impl SamplerModulationState {
    pub fn new(config: ModulationConfig) -> Self {
        let inner = ModulationState::new(config);
        Self { inner }
    }

    /// Create a new modulation matrix from this configuration.
    pub fn create_matrix(&self, sample_rate: u32) -> ModulationMatrix {
        self.inner.create_matrix(sample_rate)
    }

    /// Check if a parameter ID belongs to a modulation source.
    pub fn is_source_parameter(&self, id: FourCC) -> bool {
        self.inner.is_source_parameter(id)
    }

    /// Get modulation source descriptors for the Generator trait.
    pub fn sources(&self) -> Vec<ModulationSource> {
        self.inner.sources()
    }

    /// Get modulatable parameter IDs for the Generator trait.
    pub fn targets(&self) -> Vec<ModulationTarget> {
        self.inner.targets()
    }

    /// Apply a parameter update to all voice modulation matrices.
    pub fn apply_parameter_update(
        &mut self,
        id: FourCC,
        rate: Option<f32>,
        waveform: Option<LfoWaveform>,
        voices: &mut [SamplerVoice],
    ) -> Result<(), Error> {
        // Find which source this parameter belongs to
        for source_config in self.inner.config().sources.iter() {
            match source_config {
                ModulationSource::Lfo {
                    rate_param,
                    waveform_param,
                    ..
                } => {
                    let source_id = source_config.id();
                    let lfo_index = if let Some(ModulationSlotType::Lfo(index)) =
                        self.inner.source_slot_map().get(&source_id)
                    {
                        *index
                    } else {
                        continue;
                    };

                    if id == rate_param.id() {
                        if let Some(rate) = rate {
                            // Update all voices
                            for voice in voices {
                                voice
                                    .modulation_matrix_mut()
                                    .expect("Should have a valid modulation matrix when modulation is enabled")
                                    .update_lfo_rate(lfo_index, rate as f64);
                            }
                        }
                        return Ok(());
                    } else if id == waveform_param.id() {
                        if let Some(waveform) = waveform {
                            // Update all voices
                            for voice in voices {
                                voice
                                    .modulation_matrix_mut()
                                    .expect("Should have a valid modulation matrix when modulation is enabled")
                                    .update_lfo_waveform(lfo_index, waveform);
                            }
                        }
                        return Ok(());
                    }
                }
                ModulationSource::Envelope { .. } => {
                    panic!("Not expecting envelope modulation source for a sampler");
                }
                ModulationSource::Velocity { .. } | ModulationSource::Keytracking { .. } => {
                    // No parameters to update
                }
            }
        }

        Err(Error::ParameterError(format!(
            "Invalid/unknown modulation parameter {id}"
        )))
    }

    /// Set or update a modulation routing.
    pub fn set_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
        amount: f32,
        bipolar: bool,
    ) -> Result<(), crate::Error> {
        self.inner
            .set_modulation(matrix, source, target, amount, bipolar)
    }

    /// Clear a modulation routing.
    pub fn clear_modulation(
        &self,
        matrix: &mut ModulationMatrix,
        source: FourCC,
        target: FourCC,
    ) -> Result<(), crate::Error> {
        self.inner.clear_modulation(matrix, source, target)
    }
}

// -------------------------------------------------------------------------------------------------

/// Sampler modulation state within a SamplerVoice. Holds the modulation matrix and output buffers.
pub(crate) struct SamplerVoiceModulationState {
    matrix: ModulationMatrix,
    size: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    density: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    variation: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    spray: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    pan_spread: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    position: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
    speed: [f32; MODULATION_PROCESSOR_BLOCK_SIZE],
}

impl SamplerVoiceModulationState {
    /// Create a new voice state with the given matrix
    pub fn new(matrix: ModulationMatrix) -> Self {
        Self {
            matrix,
            size: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            density: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            variation: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            spray: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            pan_spread: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            position: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
            speed: [0.0; MODULATION_PROCESSOR_BLOCK_SIZE],
        }
    }

    /// Access to the modulation matrix.
    #[inline]
    pub fn matrix(&self) -> &ModulationMatrix {
        &self.matrix
    }

    /// Mutable access to the modulation matrix.
    #[inline]
    pub fn matrix_mut(&mut self) -> &mut ModulationMatrix {
        &mut self.matrix
    }

    /// Start modulation processing when the voice starts playing.
    pub fn start(&mut self, note: u8, volume: f32) {
        self.matrix.note_on(note, volume);
    }

    /// Stop modulation processing when the voice stops playing.
    pub fn stop(&mut self) {
        self.matrix.note_off();
    }

    /// Process modulation block into output buffers.
    ///
    /// # Arguments
    /// * `chunk_frames` - Number of frames to process (up to MODULATION_PROCESSOR_BLOCK_SIZE)
    pub fn process(&mut self, chunk_size: usize) {
        use super::Sampler;

        debug_assert!(
            chunk_size <= MODULATION_PROCESSOR_BLOCK_SIZE,
            "Frames exceeds maximum block size"
        );

        // Process modulation sources
        self.matrix.process(chunk_size);

        // Fill modulation buffers
        self.matrix.output(
            Sampler::GRAIN_SIZE.id(), //
            &mut self.size[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_DENSITY.id(), //
            &mut self.density[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_VARIATION.id(),
            &mut self.variation[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_SPRAY.id(), //
            &mut self.spray[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_PAN_SPREAD.id(),
            &mut self.pan_spread[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_POSITION.id(),
            &mut self.position[..chunk_size],
        );
        self.matrix.output(
            Sampler::GRAIN_STEP.id(), //
            &mut self.speed[..chunk_size],
        );
    }

    /// Get referneces to the last processed modulation output.
    pub fn output<'a>(&'a self, frame_count: usize) -> GranularParameterModulation<'a> {
        GranularParameterModulation {
            size: &self.size[..frame_count],
            density: &self.density[..frame_count],
            variation: &self.variation[..frame_count],
            spray: &self.spray[..frame_count],
            pan_spread: &self.pan_spread[..frame_count],
            position: &self.position[..frame_count],
            speed: &self.speed[..frame_count],
        }
    }
}
