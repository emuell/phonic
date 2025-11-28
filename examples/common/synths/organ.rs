//! Fundsp mono organ alike synth with 4 slightly detuned sine waves.
//! Does not provide any external extra parameters. To be wrapped into a [`FunDspGenerator`].

use fundsp::hacker32::*;

// -------------------------------------------------------------------------------------------------

pub fn voice_factory(gate: Shared, freq: Shared, vol: Shared, _pan: Shared) -> Box<dyn AudioUnit> {
    // Create sine waves
    let fundamental = var(&freq) >> sine();
    let harmonic_l1 = (var(&freq) * 2.01) >> sine();
    let harmonic_h1 = (var(&freq) * 0.51) >> sine();
    let harmonic_h2 = (var(&freq) * 0.249) >> sine();
    let final_sound =
        (fundamental + harmonic_l1 * 0.5 + harmonic_h1 * 0.5 + harmonic_h2 * 0.5) * 0.3;
    // Create envelope
    let envelope = var(&gate) >> adsr_live(0.001, 0.1, 0.7, 0.5);
    // Combine final sound with envelope, volume
    Box::new(final_sound * envelope * var(&vol))
}
