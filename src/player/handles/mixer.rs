use crate::{
    player::MixerId,
    source::measured::{CpuLoad, SharedMeasurementState},
};

// -------------------------------------------------------------------------------------------------

/// A handle to a mixer, which allows querying runtime properties.
///
/// Handles are `Send` and `Sync` so they can be sent across threads.
#[derive(Clone)]
pub struct MixerHandle {
    mixer_id: MixerId,
    measurement_state: SharedMeasurementState,
}

impl MixerHandle {
    pub(crate) fn new(mixer_id: MixerId, measurement_state: SharedMeasurementState) -> Self {
        Self {
            mixer_id,
            measurement_state,
        }
    }

    /// Get the mixer ID.
    pub fn id(&self) -> MixerId {
        self.mixer_id
    }

    /// Get the CPU load data for this mixer.
    pub fn cpu_load(&self) -> CpuLoad {
        self.measurement_state
            .try_lock()
            .map(|state| state.cpu_load())
            .unwrap_or_default()
    }
}
