//! Fundsp AudioNode impls

mod ahdsr;
mod fast_sine;
mod multi_osc;

pub use ahdsr::{shared_ahdsr, SharedAhdsrNode};
pub use fast_sine::{fast_sine, FastSine};
pub use multi_osc::{multi_osc, MultiOsc};
