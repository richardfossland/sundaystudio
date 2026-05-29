//! Audio output regression test.
//!
//! Guards that generated audio stays correct across refactors. Originally this
//! fingerprinted exact bytes, but that is the WRONG tool for anything derived
//! from floating-point math: `sin`/`tanh`/`powf` differ in the last bit across
//! platforms and compilers, so a byte-exact golden seeded on one OS fails on
//! another (it did — macOS vs Linux CI). See ADR-0009.
//!
//! Instead we assert signal *properties* with tolerances: the right length and
//! format, the expected peak/RMS amplitude, and the expected frequency (via
//! zero-crossing count). DSP effects are tested the same way in `src/dsp`.

use std::f32::consts::SQRT_2;

use sundaystudio_lib::audio::tone;

/// Count sign changes (zero crossings) in a sample slice.
fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

#[test]
fn test_tone_is_a_clean_440hz_half_scale_sine() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");

    let freq = 440.0;
    let sample_rate = 48_000;
    tone::write_test_tone(&path, sample_rate, freq, 1000).expect("tone writes");

    let reader = hound::WavReader::open(&path).expect("reopen WAV");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, sample_rate);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);
    assert_eq!(reader.len(), sample_rate, "one second of samples");

    // Normalise i16 → f32 in [-1, 1].
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    // Amplitude: the tone is written at -6 dBFS (0.5 full scale).
    let peak = samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!((peak - 0.5).abs() < 0.01, "peak {peak} should be ~0.5");

    // A sine's RMS is peak / √2.
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    assert!(
        (rms - 0.5 / SQRT_2).abs() < 0.01,
        "rms {rms} should be ~{}",
        0.5 / SQRT_2
    );

    // Frequency: a 440 Hz sine over 1 s has 440 cycles → ~880 zero crossings.
    let crossings = zero_crossings(&samples);
    assert!(
        (crossings as i64 - 880).abs() <= 4,
        "got {crossings} zero crossings, expected ~880 for 440 Hz"
    );
}
