//! Fundsp stereo bass synth with 3 filtered detuned sine waves and extra square wave.
//! Does not provide any external extra parameters. To be wrapped into a [`FunDspGenerator`].

use fundsp::hacker32::*;

// -------------------------------------------------------------------------------------------------

pub fn voice_factory(
    gate: Shared,
    freq: Shared,
    vol: Shared,
    panning: Shared,
) -> Box<dyn AudioUnit> {
    // Apply gate via an ADSR envelope
    let envelope = var(&gate) >> adsr_live(0.01, 0.1, 0.7, 1.0);

    // Create 3 sine modulators
    let modulator_ratio1 = 1.98;
    let modulator_ratio2 = 3.01;
    let modulator_ratio3 = 5.97;

    let mod_index1 = 3.5;
    let mod_index2 = 2.2;
    let mod_index3 = 1.8;

    let modulator1 = (var(&freq) * modulator_ratio1) >> sine();
    let modulator2 = (var(&freq) * modulator_ratio2) >> sine();
    let modulator3 = (var(&freq) * modulator_ratio3) >> sine();
    let mod_amount1 = modulator1 * var(&freq) * mod_index1 * envelope.clone();

    let mod_amount2 = modulator2 * var(&freq) * mod_index2 * envelope.clone();
    let mod_amount3 = modulator3 * var(&freq) * mod_index3 * envelope.clone() * 0.75;
    let carrier1 = (var(&freq) + mod_amount1.clone()) >> sine();

    let carrier2 = ((var(&freq) * 1.003) + mod_amount2 * 0.7) >> sine();
    let carrier3 = ((var(&freq) * 0.997) + mod_amount3) >> sine();

    // Add an extra square wave for brightness
    let square_carrier = ((var(&freq) + mod_amount1.clone() * 0.4) >> square()) * 0.3;

    // Mix all carriers with different amplitudes and a LP
    let fm_sound = ((carrier1 * 0.5 + carrier2 * 0.35 + carrier3 * 0.15 + square_carrier) * 0.5)
        >> lowpass_hz(1600.0, 0.6);

    // Add high-frequency resonance
    let resonance = (var(&freq) * 2.0) >> sine() >> highpass_hz(1000.0, 5.0);
    let final_sound = fm_sound * 0.6 + resonance * envelope.clone() * 0.4;

    // Combine final sound with envelope, volume and panning (which makes it stereo)
    Box::new(((final_sound * envelope * var(&vol)) | var(&panning)) >> panner())
}
