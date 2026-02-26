use std::sync::Arc;

use crate::{
    player::MixerId,
    source::{
        measured::{CpuLoad, SharedCpuLoadState},
        metered::{AudioLevel, SharedAudioLevelState},
    },
};

// -------------------------------------------------------------------------------------------------

/// A handle to a mixer, which allows querying runtime properties.
///
/// Handles are `Send` and `Sync` so they can be sent across threads.
#[derive(Clone)]
pub struct MixerHandle {
    mixer_id: MixerId,
    measurement_state: Option<SharedCpuLoadState>,
    metering_state: Option<SharedAudioLevelState>,
}

impl MixerHandle {
    pub(crate) fn new(
        mixer_id: MixerId,
        measurement_state: Option<SharedCpuLoadState>,
        metering_state: Option<SharedAudioLevelState>,
    ) -> Self {
        Self {
            mixer_id,
            measurement_state,
            metering_state,
        }
    }

    /// Get the mixer ID.
    pub fn id(&self) -> MixerId {
        self.mixer_id
    }

    /// Get the CPU load data for this mixer.
    ///
    /// Only available when CPU measurement was enabled in the playback options
    /// and the player's [`PlayerConfig`](crate::PlayerConfig).
    pub fn cpu_load(&self) -> Option<CpuLoad> {
        self.measurement_state
            .as_ref()
            .and_then(|s| s.try_lock().ok())
            .map(|state| state.cpu_load())
    }

    /// Get the CPU load data for this mixer.
    ///
    /// Only available when CPU measurement is enabled in the player's [`PlayerConfig`](crate::PlayerConfig).
    pub fn cpu_load_state(&self) -> Option<SharedCpuLoadState> {
        self.measurement_state.as_ref().map(Arc::clone)
    }

    /// Get the current audio level for this mixer.
    ///
    /// Only available when audio metering is enabled in the player's [`PlayerConfig`](crate::PlayerConfig).
    pub fn audio_level(&self) -> Option<AudioLevel> {
        self.metering_state
            .as_ref()
            .and_then(|s| s.try_lock().ok())
            .map(|state| state.audio_level().clone())
    }

    /// Get the shared audio level state for this mixer, if metering is enabled.
    ///
    /// Only available when audio metering is enabled in the player's [`PlayerConfig`](crate::PlayerConfig).
    pub fn audio_level_state(&self) -> Option<SharedAudioLevelState> {
        self.metering_state.as_ref().map(Arc::clone)
    }
}
