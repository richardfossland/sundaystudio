//! De-esser. A high-pass *detector* listens to the sibilance band (the "s" and
//! "t" energy, typically 4–9 kHz); when that band gets harsh, the signal is
//! ducked for those brief moments. Detection is frequency-conscious so steady
//! vowels and lows never trigger it; the reduction itself is broadband, which
//! keeps the design phase-coherent (no comb filtering from band recombination)
//! and is how most simple de-essers work. A true split-band version can come
//! later if users want to preserve lows during a sustained "sssss".

use super::biquad::{Biquad, FilterType};
use super::{db_to_gain, gain_to_db, time_coeff, Effect};

#[derive(Debug, Clone)]
pub struct DeEsser {
    /// Crossover / sibilance target frequency (Hz), typically 4–9 kHz.
    pub freq: f32,
    /// High-band level above which reduction kicks in (dBFS).
    pub threshold_db: f32,
    /// Compression ratio applied to the high band.
    pub ratio: f32,
    pub bypass: bool,

    sample_rate: f32,
    hp: Biquad,
    attack_coeff: f32,
    release_coeff: f32,
    env: f32,
}

impl Default for DeEsser {
    fn default() -> Self {
        Self {
            freq: 6500.0,
            // Transparent default: threshold so high it never engages.
            threshold_db: 0.0,
            ratio: 1.0,
            bypass: false,
            sample_rate: 48_000.0,
            hp: Biquad::default(),
            attack_coeff: 0.0,
            release_coeff: 0.0,
            env: 0.0,
        }
    }
}

impl DeEsser {
    /// A working de-esser for a bright voice.
    pub fn voice(threshold_db: f32) -> Self {
        Self {
            threshold_db,
            ratio: 4.0,
            ..Default::default()
        }
    }
}

impl Effect for DeEsser {
    fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.hp
            .set(FilterType::HighPass, sample_rate, self.freq, 0.707, 0.0);
        self.hp.reset();
        self.attack_coeff = time_coeff(0.5, sample_rate);
        self.release_coeff = time_coeff(40.0, sample_rate);
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass || self.ratio <= 1.0 {
            return;
        }
        let slope = 1.0 / self.ratio - 1.0;
        for s in block.iter_mut() {
            // Detect on the sibilance band only (the high-pass is a side-chain).
            let detect = self.hp.process_sample(*s).abs();
            let coeff = if detect > self.env {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.env = detect + coeff * (self.env - detect);

            let level_db = gain_to_db(self.env);
            let reduction = if level_db.is_finite() && level_db > self.threshold_db {
                slope * (level_db - self.threshold_db)
            } else {
                0.0
            };
            // Broadband duck for the brief sibilant moment.
            *s *= db_to_gain(reduction);
        }
    }

    fn reset(&mut self) {
        self.hp.reset();
        self.env = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{rms, sine};

    const SR: f32 = 48_000.0;

    fn gain_db(de: &mut DeEsser, freq: f32, amp: f32) -> f32 {
        let n = SR as usize;
        let input: Vec<f32> = sine(freq, SR, n).iter().map(|s| s * amp).collect();
        let mut buf = input.clone();
        de.process(&mut buf);
        let half = n / 2;
        gain_to_db(rms(&buf[half..]) / rms(&input[half..]))
    }

    #[test]
    fn reduces_sibilant_highs_not_lows() {
        let mut de = DeEsser::voice(-30.0);
        de.prepare(SR);
        // Loud 8 kHz (sibilance) is pulled down.
        assert!(gain_db(&mut de, 8000.0, 0.9) < -3.0);
        de.reset();
        // A low/mid vowel is left alone.
        assert!(gain_db(&mut de, 300.0, 0.9).abs() < 1.0);
    }

    #[test]
    fn transparent_by_default() {
        let mut de = DeEsser::default();
        de.prepare(SR);
        assert!(gain_db(&mut de, 8000.0, 0.5).abs() < 0.5);
    }
}
