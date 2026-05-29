//! Biquad filter — the workhorse behind the EQ and the de-esser's crossover.
//!
//! Coefficients follow the RBJ "Audio EQ Cookbook"; processing is Direct Form I.
//! `Biquad` is real-time safe: designing coefficients does a little trig up
//! front (in `prepare`), and `process_sample` is a handful of mul-adds.

use std::f32::consts::PI;

/// The kinds of second-order section we need.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterType {
    LowPass,
    HighPass,
    Peaking,
    LowShelf,
    HighShelf,
    BandPass,
}

/// Normalised biquad coefficients (a0 divided out).
#[derive(Debug, Clone, Copy, Default)]
struct Coeffs {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

/// A single second-order section with its sample memory.
#[derive(Debug, Clone, Default)]
pub struct Biquad {
    c: Coeffs,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    /// Design coefficients (RBJ cookbook). `gain_db` is used only by the
    /// peaking and shelf types. `q` controls bandwidth/steepness.
    pub fn set(&mut self, ftype: FilterType, sample_rate: f32, freq: f32, q: f32, gain_db: f32) {
        let freq = freq.clamp(10.0, sample_rate * 0.49);
        let q = q.max(1e-4);
        let w0 = 2.0 * PI * freq / sample_rate;
        let (sin, cos) = (w0.sin(), w0.cos());
        let alpha = sin / (2.0 * q);
        let a = 10.0_f32.powf(gain_db / 40.0);

        let (b0, b1, b2, a0, a1, a2) = match ftype {
            FilterType::LowPass => (
                (1.0 - cos) / 2.0,
                1.0 - cos,
                (1.0 - cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterType::HighPass => (
                (1.0 + cos) / 2.0,
                -(1.0 + cos),
                (1.0 + cos) / 2.0,
                1.0 + alpha,
                -2.0 * cos,
                1.0 - alpha,
            ),
            FilterType::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cos, 1.0 - alpha),
            FilterType::Peaking => (
                1.0 + alpha * a,
                -2.0 * cos,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cos,
                1.0 - alpha / a,
            ),
            FilterType::LowShelf => {
                let sa = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * cos + sa),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                    a * ((a + 1.0) - (a - 1.0) * cos - sa),
                    (a + 1.0) + (a - 1.0) * cos + sa,
                    -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                    (a + 1.0) + (a - 1.0) * cos - sa,
                )
            }
            FilterType::HighShelf => {
                let sa = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * cos + sa),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                    a * ((a + 1.0) + (a - 1.0) * cos - sa),
                    (a + 1.0) - (a - 1.0) * cos + sa,
                    2.0 * ((a - 1.0) - (a + 1.0) * cos),
                    (a + 1.0) - (a - 1.0) * cos - sa,
                )
            }
        };

        self.c = Coeffs {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: a1 / a0,
            a2: a2 / a0,
        };
    }

    /// Clear the filter memory (call when the signal restarts).
    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }

    #[inline]
    pub fn process_sample(&mut self, x: f32) -> f32 {
        let c = &self.c;
        let y = c.b0 * x + c.b1 * self.x1 + c.b2 * self.x2 - c.a1 * self.y1 - c.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }

    #[inline]
    pub fn process(&mut self, block: &mut [f32]) {
        for s in block.iter_mut() {
            *s = self.process_sample(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{gain_db_at_filter, sine};

    const SR: f32 = 48_000.0;

    #[test]
    fn peaking_boost_is_local_to_its_frequency() {
        let mut f = Biquad::default();
        f.set(FilterType::Peaking, SR, 1000.0, 1.0, 6.0);
        // ~+6 dB at the centre, ~0 dB two decades away.
        assert!((gain_db_at_filter(&mut f, SR, 1000.0) - 6.0).abs() < 1.0);
        f.reset();
        assert!(gain_db_at_filter(&mut f, SR, 60.0).abs() < 1.0);
    }

    #[test]
    fn highpass_attenuates_lows_passes_highs() {
        let mut f = Biquad::default();
        f.set(FilterType::HighPass, SR, 200.0, 0.707, 0.0);
        assert!(gain_db_at_filter(&mut f, SR, 40.0) < -10.0);
        f.reset();
        assert!(gain_db_at_filter(&mut f, SR, 4000.0).abs() < 1.0);
    }

    #[test]
    fn lowpass_attenuates_highs() {
        let mut f = Biquad::default();
        f.set(FilterType::LowPass, SR, 1000.0, 0.707, 0.0);
        assert!(gain_db_at_filter(&mut f, SR, 8000.0) < -10.0);
        f.reset();
        assert!(gain_db_at_filter(&mut f, SR, 100.0).abs() < 1.0);
    }

    #[test]
    fn high_shelf_lifts_only_the_top() {
        let mut f = Biquad::default();
        f.set(FilterType::HighShelf, SR, 4000.0, 0.707, 6.0);
        assert!(gain_db_at_filter(&mut f, SR, 100.0).abs() < 1.0);
        f.reset();
        assert!(gain_db_at_filter(&mut f, SR, 12000.0) > 4.0);
    }

    #[test]
    fn is_stable_no_nan_blowup() {
        let mut f = Biquad::default();
        f.set(FilterType::Peaking, SR, 3000.0, 2.0, 12.0);
        let mut buf = sine(3000.0, SR, 48_000);
        f.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }
}
