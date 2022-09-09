pub mod actor;
pub mod decoder;
pub mod resampler;

use std::sync::atomic::{AtomicUsize, Ordering};

// -------------------------------------------------------------------------------------------------

/// Generates a unique usize number, by simply counting atomically upwards from 1.
pub fn unique_usize_id() -> usize {
    static FILE_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);
    FILE_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}
