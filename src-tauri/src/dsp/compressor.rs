//! Feed-forward compressor with a soft knee and optional auto makeup gain —
//! the everyday tool for evening out a speaker's dynamics. A level detector
//! (attack/release-smoothed peak) drives a dB-domain gain computer.

use super::{db_to_gain, gain_to_db, time_coeff, Effect};

#[derive(Debug, Clone)]
pub struct Compressor {
    pub threshold_db: f32,
    pub ratio: f32,
    /// Soft-knee width in dB (0 = hard knee).
    pub knee_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
    pub makeup_db: f32,
    /// When true, makeup is computed automatically and `makeup_db` is ignored.
    pub auto_makeup: bool,
    pub bypass: bool,

    sample_rate: f32,
    attack_coeff: f32,
    release_coeff: f32,
    env: f32,
}

impl Default for Compressor {
    fn default() -> Self {
        // Transparent: 1:1 ratio does nothing.
        Self {
            threshold_db: -18.0,
            ratio: 1.0,
            knee_db: 6.0,
            attack_ms: 10.0,
            release_ms: 100.0,
            makeup_db: 0.0,
            auto_makeup: false,
            bypass: false,
            sample_rate: 48_000.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            env: 0.0,
        }
    }
}

impl Compressor {
    /// A gentle voice compressor.
    pub fn voice(threshold_db: f32, ratio: f32) -> Self {
        Self {
            threshold_db,
            ratio,
            auto_makeup: true,
            ..Default::default()
        }
    }

    /// Gain change (dB, ≤ 0) the computer applies at a given input level.
    fn gain_computer_db(&self, level_db: f32) -> f32 {
        let slope = 1.0 / self.ratio - 1.0; // ≤ 0
        let diff = level_db - self.threshold_db;
        if self.knee_db > 0.0 && 2.0 * diff.abs() <= self.knee_db {
            slope * (diff + self.knee_db / 2.0).powi(2) / (2.0 * self.knee_db)
        } else if diff > 0.0 {
            slope * diff
        } else {
            0.0
        }
    }

    /// Auto makeup: half-compensate the reduction a 0 dBFS signal would see, so
    /// output loudness roughly tracks input without pumping perception.
    fn makeup(&self) -> f32 {
        if self.auto_makeup {
            -self.gain_computer_db(0.0) * 0.5
        } else {
            self.makeup_db
        }
    }
}

impl Effect for Compressor {
    fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.attack_coeff = time_coeff(self.attack_ms, sample_rate);
        self.release_coeff = time_coeff(self.release_ms, sample_rate);
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass || self.ratio <= 1.0 {
            // Still apply explicit makeup if asked, but skip the computer.
            if !self.bypass && self.makeup_db != 0.0 && !self.auto_makeup {
                let g = db_to_gain(self.makeup_db);
                for s in block.iter_mut() {
                    *s *= g;
                }
            }
            return;
        }

        let makeup = self.makeup();
        for s in block.iter_mut() {
            let rect = s.abs();
            let coeff = if rect > self.env {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.env = rect + coeff * (self.env - rect);

            let level_db = gain_to_db(self.env);
            let reduction = if level_db.is_finite() {
                self.gain_computer_db(level_db)
            } else {
                0.0
            };
            *s *= db_to_gain(reduction + makeup);
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{rms, sine};

    const SR: f32 = 48_000.0;

    fn gain_db(comp: &mut Compressor, amp: f32) -> f32 {
        let n = SR as usize;
        let input: Vec<f32> = sine(220.0, SR, n).iter().map(|s| s * amp).collect();
        let mut buf = input.clone();
        comp.process(&mut buf);
        let half = n / 2;
        gain_to_db(rms(&buf[half..]) / rms(&input[half..]))
    }

    #[test]
    fn unity_ratio_is_transparent() {
        let mut c = Compressor::default();
        c.prepare(SR);
        assert!(gain_db(&mut c, 1.0).abs() < 0.3);
    }

    #[test]
    fn compresses_above_threshold_not_below() {
        // 4:1 at -20 dBFS, no makeup.
        let mut c = Compressor {
            threshold_db: -20.0,
            ratio: 4.0,
            auto_makeup: false,
            ..Default::default()
        };
        c.prepare(SR);
        // Loud (0 dBFS) gets pulled down clearly.
        assert!(gain_db(&mut c, 1.0) < -8.0);
        c.reset();
        // Quiet (~-40 dBFS) is essentially untouched.
        assert!(gain_db(&mut c, 0.01).abs() < 1.0);
    }

    #[test]
    fn auto_makeup_lifts_output() {
        let mut without = Compressor {
            threshold_db: -20.0,
            ratio: 4.0,
            auto_makeup: false,
            ..Default::default()
        };
        without.prepare(SR);
        let mut with = Compressor::voice(-20.0, 4.0);
        with.prepare(SR);
        assert!(gain_db(&mut with, 1.0) > gain_db(&mut without, 1.0));
    }
}
