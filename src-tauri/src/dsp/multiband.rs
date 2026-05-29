//! Three-band compressor for the master bus (Phase 4.2b).
//!
//! A single compressor on a full mix lets a loud low-end pump the highs (every
//! bass thump ducks the whole signal). Splitting into low / mid / high and
//! compressing each independently keeps the voice's body, presence and air
//! moving on their own — gentle glue, not a sledgehammer.
//!
//! The split uses 4th-order Linkwitz-Riley crossovers (two cascaded Butterworth
//! biquads each): low + high of an LR crossover recombine flat in magnitude, so
//! with the band compressors at unity the chain is transparent. Bands run
//! through the existing [`Compressor`](super::compressor::Compressor), then sum.
//!
//! The per-block band buffers are reusable scratch that only (re)allocates when
//! the block grows — steady-state (fixed block size) processing allocates
//! nothing, honouring the engine's real-time discipline.

use super::biquad::{Biquad, FilterType};
use super::compressor::Compressor;
use super::Effect;

/// Linkwitz-Riley 4th-order section: two identical Butterworth biquads cascaded.
#[derive(Debug, Clone, Default)]
struct Lr4 {
    a: Biquad,
    b: Biquad,
}

impl Lr4 {
    const BUTTERWORTH_Q: f32 = std::f32::consts::FRAC_1_SQRT_2; // 0.7071

    fn set(&mut self, ftype: FilterType, sample_rate: f32, freq: f32) {
        self.a.set(ftype, sample_rate, freq, Self::BUTTERWORTH_Q, 0.0);
        self.b.set(ftype, sample_rate, freq, Self::BUTTERWORTH_Q, 0.0);
    }

    #[inline]
    fn process_sample(&mut self, x: f32) -> f32 {
        self.b.process_sample(self.a.process_sample(x))
    }

    fn reset(&mut self) {
        self.a.reset();
        self.b.reset();
    }
}

/// A 3-band compressor split at `low_xover` and `high_xover` Hz.
#[derive(Debug, Clone)]
pub struct MultibandCompressor {
    pub low_xover_hz: f32,
    pub high_xover_hz: f32,
    pub low: Compressor,
    pub mid: Compressor,
    pub high: Compressor,
    pub bypass: bool,

    // Crossover filters: f1 splits low/rest, f2 splits rest into mid/high.
    lp1: Lr4,
    hp1: Lr4,
    lp2: Lr4,
    hp2: Lr4,

    // Reusable per-band scratch (grows with the block, never shrinks).
    low_buf: Vec<f32>,
    mid_buf: Vec<f32>,
    high_buf: Vec<f32>,
}

impl Default for MultibandCompressor {
    fn default() -> Self {
        Self {
            low_xover_hz: 250.0,
            high_xover_hz: 3000.0,
            // Transparent until configured (unity ratio = no compression).
            low: Compressor::default(),
            mid: Compressor::default(),
            high: Compressor::default(),
            bypass: false,
            lp1: Lr4::default(),
            hp1: Lr4::default(),
            lp2: Lr4::default(),
            hp2: Lr4::default(),
            low_buf: Vec::new(),
            mid_buf: Vec::new(),
            high_buf: Vec::new(),
        }
    }
}

impl MultibandCompressor {
    /// A gentle voice-glue setting: light, faster mid control, easy low/high.
    pub fn voice() -> Self {
        Self {
            low: Compressor::voice(-24.0, 2.0),
            mid: Compressor::voice(-20.0, 2.5),
            high: Compressor::voice(-22.0, 2.0),
            ..Default::default()
        }
    }
}

impl Effect for MultibandCompressor {
    fn prepare(&mut self, sample_rate: f32) {
        self.lp1.set(FilterType::LowPass, sample_rate, self.low_xover_hz);
        self.hp1.set(FilterType::HighPass, sample_rate, self.low_xover_hz);
        self.lp2.set(FilterType::LowPass, sample_rate, self.high_xover_hz);
        self.hp2.set(FilterType::HighPass, sample_rate, self.high_xover_hz);
        self.low.prepare(sample_rate);
        self.mid.prepare(sample_rate);
        self.high.prepare(sample_rate);
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass {
            return;
        }
        let n = block.len();
        self.low_buf.resize(n, 0.0);
        self.mid_buf.resize(n, 0.0);
        self.high_buf.resize(n, 0.0);

        // Split each sample into the three bands via the LR crossovers.
        for (i, &x) in block.iter().enumerate() {
            let low = self.lp1.process_sample(x);
            let rest = self.hp1.process_sample(x);
            let mid = self.lp2.process_sample(rest);
            let high = self.hp2.process_sample(rest);
            self.low_buf[i] = low;
            self.mid_buf[i] = mid;
            self.high_buf[i] = high;
        }

        // Compress each band on its own dynamics.
        self.low.process(&mut self.low_buf);
        self.mid.process(&mut self.mid_buf);
        self.high.process(&mut self.high_buf);

        // Recombine.
        for (i, x) in block.iter_mut().enumerate() {
            *x = self.low_buf[i] + self.mid_buf[i] + self.high_buf[i];
        }
    }

    fn reset(&mut self) {
        self.lp1.reset();
        self.hp1.reset();
        self.lp2.reset();
        self.hp2.reset();
        self.low.reset();
        self.mid.reset();
        self.high.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::gain_db_at;

    const SR: f32 = 48_000.0;

    #[test]
    fn unity_bands_reconstruct_roughly_flat() {
        // With all bands at unity ratio, the LR split + sum is magnitude-flat.
        let mut mb = MultibandCompressor::default();
        mb.prepare(SR);
        for freq in [100.0, 500.0, 1000.0, 3000.0, 8000.0] {
            mb.reset();
            let g = gain_db_at(&mut mb, SR, freq);
            assert!(g.abs() < 1.5, "reconstruction off by {g} dB at {freq} Hz");
        }
    }

    #[test]
    fn compresses_a_loud_band_and_stays_finite() {
        let mut mb = MultibandCompressor::voice();
        mb.prepare(SR);
        // A loud low tone should be pulled down by the low-band compressor.
        let g = gain_db_at(&mut mb, SR, 120.0);
        assert!(g < 0.0, "expected low-band gain reduction, got {g} dB");

        mb.reset();
        let mut buf = crate::dsp::testutil::sine(1000.0, SR, 4800);
        mb.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }

    #[test]
    fn bypass_passes_through() {
        let mut mb = MultibandCompressor::voice();
        mb.prepare(SR);
        mb.bypass = true;
        assert!(gain_db_at(&mut mb, SR, 1000.0).abs() < 0.01);
    }
}
