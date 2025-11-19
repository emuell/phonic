use crate::{
    source::{
        amplified::AmplifiedSource, file::preloaded::PreloadedFileSource,
        mapped::ChannelMappedSource, panned::PannedSource, Source, SourceTime,
    },
    utils::{
        ahdsr::{AhdsrEnvelope, AhdsrParameters, AhdsrStage},
        buffer::{scale_buffer, InterleavedBufferMut},
        speed_from_note,
    },
    PlaybackId,
};

// -------------------------------------------------------------------------------------------------

/// Wrapped sampler voice types
type SamplerVoiceAmplifiedSource = AmplifiedSource<ChannelMappedSource<PreloadedFileSource>>;
type SamplerVoicePannedSource = PannedSource<SamplerVoiceAmplifiedSource>;
type SamplerVoiceSource = SamplerVoicePannedSource;

// -------------------------------------------------------------------------------------------------

pub struct SamplerVoice {
    playback_id: Option<PlaybackId>,
    source: SamplerVoiceSource,
    envelope: AhdsrEnvelope,
    release_start_frame: Option<u64>,
}

impl SamplerVoice {
    pub fn new(file_source: PreloadedFileSource, channel_count: usize) -> Self {
        let playback_id = None;

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

        Self {
            playback_id,
            source,
            envelope,
            release_start_frame,
        }
    }

    #[inline(always)]
    /// This voice's note playback id. None, when stopped.
    pub fn playback_id(&self) -> Option<usize> {
        self.playback_id
    }

    #[inline(always)]
    /// Is this voice currently playing something?
    pub fn is_active(&self) -> bool {
        self.playback_id.is_some()
    }

    #[inline(always)]
    /// Sample frame time when voice started its release mode.
    pub fn in_release_stage(&self) -> bool {
        self.envelope.stage() == AhdsrStage::Release
    }

    #[inline(always)]
    /// Sample frame time when voice started its release mode.
    pub fn release_start_frame(&self) -> Option<u64> {
        self.release_start_frame
    }

    pub fn start(
        &mut self,
        note_playback_id: PlaybackId,
        note: u8,
        volume: f32,
        panning: f32,
        envelope_parameters: &Option<AhdsrParameters>,
    ) {
        // Reset a probably recycled file source
        self.reset();
        // Set initial speed, volume and pan
        self.file_source().set_speed(speed_from_note(note), None);
        self.amplified_source().set_volume(volume);
        self.panned_source().set_panning(panning);
        // Start envelope
        if let Some(envelope_parameters) = envelope_parameters {
            self.envelope.note_on(envelope_parameters, 1.0);
        }
        self.playback_id = Some(note_playback_id)
    }
    /// Stop the voice and start fadeouts .
    pub fn stop(
        &mut self,
        envelope_parameters: &Option<AhdsrParameters>,
        current_sample_frame: u64,
    ) {
        if self.is_active() {
            self.release_start_frame = Some(current_sample_frame);
            if let Some(envelope_parameters) = envelope_parameters {
                self.envelope.note_off(envelope_parameters);
            } else {
                self.file_source().stop();
            }
        }
    }

    /// Stop & reset the voice to finish actual and prepare new playback.
    pub fn reset(&mut self) {
        // reset sources
        if self.is_active() {
            self.file_source().reset();
            self.playback_id = None;
        }
        // reset release start time
        self.release_start_frame = None;
    }

    /// Set a new playback speed value with optional glide.
    pub fn set_speed(&mut self, speed: f64, glide: Option<f32>) {
        self.file_source().set_speed(speed, glide);
    }

    /// Set a new volume value.
    pub fn set_volume(&mut self, volume: f32) {
        self.amplified_source().set_volume(volume);
    }

    /// Set a new panning value.
    pub fn set_panning(&mut self, panning: f32) {
        self.panned_source().set_panning(panning);
    }

    /// Write source and apply envelope, if set.
    pub fn process(
        &mut self,
        output: &mut [f32],
        channel_count: usize,
        envelope_parameters: &Option<AhdsrParameters>,
        time: &SourceTime,
    ) -> usize {
        debug_assert!(self.is_active(), "Only active voices need to process");

        // Write source
        let written = self.source.write(output, time);

        // Apply envelope to the voice output
        if let Some(envelope_parameters) = envelope_parameters {
            debug_assert!(self.envelope.stage() != AhdsrStage::Idle);
            let mut output = &mut output[..written];
            if self.envelope.stage() == AhdsrStage::Sustain {
                // no need to run the envelope per frame in sustain state
                scale_buffer(output, self.envelope.output());
            } else {
                for frame in output.frames_mut(channel_count) {
                    let envelope_value = self.envelope.process(envelope_parameters);
                    for sample in frame {
                        *sample *= envelope_value;
                    }
                }
            }
        }

        // Check if voice finished playback or envelope finished
        if self.source.is_exhausted()
            || (envelope_parameters.is_some() && self.envelope.stage() == AhdsrStage::Idle)
        {
            self.reset();
        }

        written
    }

    #[inline]
    fn panned_source(&mut self) -> &mut SamplerVoicePannedSource {
        &mut self.source
    }

    #[inline]
    fn amplified_source(&mut self) -> &mut SamplerVoiceAmplifiedSource {
        self.source.input_source_mut()
    }

    #[inline]
    fn file_source(&mut self) -> &mut PreloadedFileSource {
        self.source
            .input_source_mut()
            .input_source_mut()
            .input_source_mut()
    }
}
