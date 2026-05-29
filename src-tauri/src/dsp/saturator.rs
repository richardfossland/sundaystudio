//! Soft saturator — gentle `tanh` waveshaping for analog-style warmth (think
//! tape, not distortion). Useful for thickening a thin voice. The shaper is
//! normalised by `tanh(drive)` so it stays near unity at low drive, and a
//! dry/wet `mix` keeps it subtle.

use super::Effect;

#[derive(Debug, Clone)]
pub struct Saturator {
    /// Drive amount (> 0). Higher = more harmonics / softer peaks.
    pub drive: f32,
    /// Dry/wet blend, 0.0 (clean) .. 1.0 (fully shaped).
    pub mix: f32,
    pub bypass: bool,
}

impl Default for Saturator {
    fn default() -> Self {
        // Transparent-ish: very low drive, no wet mix.
        Self {
            drive: 1.0,
            mix: 0.0,
            bypass: false,
        }
    }
}

impl Saturator {
    pub fn warm(drive: f32, mix: f32) -> Self {
        Self {
            drive: drive.max(0.01),
            mix: mix.clamp(0.0, 1.0),
            bypass: false,
        }
    }

    #[inline]
    fn shape(&self, x: f32) -> f32 {
        // Normalised soft clip: at small drive this ≈ x (transparent).
        let norm = self.drive.tanh();
        if norm.abs() < 1e-6 {
            x
        } else {
            (self.drive * x).tanh() / norm
        }
    }
}

impl Effect for Saturator {
    fn prepare(&mut self, _sample_rate: f32) {}

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass || self.mix <= 0.0 {
            return;
        }
        let mix = self.mix.clamp(0.0, 1.0);
        for s in block.iter_mut() {
            let wet = self.shape(*s);
            *s = *s * (1.0 - mix) + wet * mix;
        }
    }

    fn reset(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{peak, sine};

    const SR: f32 = 48_000.0;

    #[test]
    fn transparent_when_dry() {
        let mut s = Saturator::default();
        s.prepare(SR);
        let mut buf = sine(440.0, SR, 1000);
        let before = buf.clone();
        s.process(&mut buf);
        assert!(buf
            .iter()
            .zip(before.iter())
            .all(|(a, b)| (a - b).abs() < 1e-6));
    }

    #[test]
    fn soft_clips_loud_peaks() {
        // Strong drive, full wet: a full-scale signal's peak is tamed and the
        // transfer stays bounded (no hard overs).
        let mut s = Saturator::warm(4.0, 1.0);
        s.prepare(SR);
        let mut buf: Vec<f32> = sine(440.0, SR, 1000).iter().map(|x| x * 1.0).collect();
        s.process(&mut buf);
        assert!(peak(&buf) <= 1.001);
        assert!(buf.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn shape_is_monotonic() {
        let s = Saturator::warm(5.0, 1.0);
        let mut prev = f32::NEG_INFINITY;
        let mut x = -1.0;
        while x <= 1.0 {
            let y = s.shape(x);
            assert!(y >= prev, "transfer must be non-decreasing");
            prev = y;
            x += 0.01;
        }
    }
}
