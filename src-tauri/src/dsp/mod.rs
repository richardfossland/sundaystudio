//! Built-in DSP effects (Phase 4.1). Bundled, real-time-safe, no third-party
//! plugin hosting — that restraint is what keeps us simpler than GarageBand.
//!
//! Every effect implements [`Effect`]: `prepare` (cache sample-rate-dependent
//! coefficients), `process` (in-place on a mono block, NO allocation), `reset`
//! (clear filter/envelope state). Effects are bypassable and have a transparent
//! `Default`. The voice chain (gate → EQ → de-esser → compressor → saturator)
//! and factory presets live in `chain`.
//!
//! These are tested by signal *properties* (frequency response, gain reduction)
//! rather than byte-exact goldens: floating-point trig/`tanh` differ subtly
//! across platforms, so exact fingerprints would be flaky in CI. Property tests
//! with sensible tolerances are both robust and meaningful.

pub mod biquad;
pub mod chain;
pub mod compressor;
pub mod deesser;
pub mod eq;
pub mod gate;
pub mod limiter;
pub mod loudness;
pub mod master;
pub mod multiband;
pub mod saturator;

/// In-place, real-time-safe audio processor over a mono block.
pub trait Effect {
    /// Cache anything that depends on the sample rate. Called before playback
    /// and whenever the sample rate changes.
    fn prepare(&mut self, sample_rate: f32);
    /// Process a block in place. Must not allocate or block.
    fn process(&mut self, block: &mut [f32]);
    /// Clear internal state (filter memory, envelopes).
    fn reset(&mut self);
}

/// Linear gain from decibels: `10^(db/20)`.
#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10.0_f32.powf(db / 20.0)
}

/// Decibels from a linear gain. Returns -inf at/below zero.
#[inline]
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * gain.log10()
    }
}

/// First-order smoothing coefficient for an attack/release time constant.
/// `time_ms` is the time to reach ~63% of a step; returns the per-sample
/// retention factor in [0, 1). A zero/negative time gives 0 (instant).
#[inline]
pub fn time_coeff(time_ms: f32, sample_rate: f32) -> f32 {
    if time_ms <= 0.0 || sample_rate <= 0.0 {
        0.0
    } else {
        (-1.0 / (time_ms * 0.001 * sample_rate)).exp()
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    //! Shared signal helpers for DSP property tests.
    use std::f32::consts::TAU;

    /// Generate `n` samples of a unit-amplitude sine at `freq` Hz.
    pub fn sine(freq: f32, sample_rate: f32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| (freq * TAU * i as f32 / sample_rate).sin())
            .collect()
    }

    /// RMS of a slice.
    pub fn rms(x: &[f32]) -> f32 {
        if x.is_empty() {
            return 0.0;
        }
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Peak absolute value.
    pub fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, &s| m.max(s.abs()))
    }

    /// Steady-state gain (dB) an effect applies to a sine at `freq`: process a
    /// half-second tone and compare output RMS to input RMS over the settled
    /// second half (skipping the transient).
    pub fn gain_db_at<F: super::Effect>(effect: &mut F, sample_rate: f32, freq: f32) -> f32 {
        let n = (sample_rate * 0.5) as usize;
        let input = sine(freq, sample_rate, n);
        let mut buf = input.clone();
        effect.process(&mut buf);
        settled_gain_db(&input, &buf)
    }

    /// Same, but for a raw [`super::biquad::Biquad`] (not an `Effect`).
    pub fn gain_db_at_filter(
        filter: &mut super::biquad::Biquad,
        sample_rate: f32,
        freq: f32,
    ) -> f32 {
        let n = (sample_rate * 0.5) as usize;
        let input = sine(freq, sample_rate, n);
        let mut buf = input.clone();
        filter.process(&mut buf);
        settled_gain_db(&input, &buf)
    }

    /// dB ratio of output to input RMS over the settled second half.
    fn settled_gain_db(input: &[f32], output: &[f32]) -> f32 {
        let half = input.len() / 2;
        super::gain_to_db(rms(&output[half..]) / rms(&input[half..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_gain_round_trip() {
        assert!((db_to_gain(0.0) - 1.0).abs() < 1e-6);
        assert!((db_to_gain(6.0206) - 2.0).abs() < 1e-3);
        assert!((gain_to_db(2.0) - 6.0206).abs() < 1e-3);
        assert_eq!(gain_to_db(0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn time_coeff_bounds() {
        assert_eq!(time_coeff(0.0, 48_000.0), 0.0);
        let c = time_coeff(10.0, 48_000.0);
        assert!(c > 0.0 && c < 1.0);
    }
}
