//! Noise gate. Critical for multi-mic setups: when a guest isn't talking, their
//! mic (full of room and bleed) is pulled down by `range` dB so it doesn't muddy
//! the mix. A peak detector with a quick release drives an attack/hold/release
//! gain envelope.

use super::{db_to_gain, time_coeff, Effect};

/// Noise gate parameters + state.
#[derive(Debug, Clone)]
pub struct Gate {
    /// Open above this input level (dBFS).
    pub threshold_db: f32,
    /// Time to open once the signal crosses the threshold.
    pub attack_ms: f32,
    /// Stay open this long after dropping below threshold.
    pub hold_ms: f32,
    /// Time to close.
    pub release_ms: f32,
    /// Attenuation when fully closed (dB, negative). 0 = no gating.
    pub range_db: f32,
    pub bypass: bool,

    sample_rate: f32,
    attack_coeff: f32,
    release_coeff: f32,
    det_release: f32,
    hold_samples: u32,
    // state
    env: f32,
    gain: f32,
    hold_counter: u32,
}

impl Default for Gate {
    fn default() -> Self {
        // Transparent default: threshold so low it's always open, no range.
        Self {
            threshold_db: -100.0,
            attack_ms: 1.0,
            hold_ms: 50.0,
            release_ms: 100.0,
            range_db: 0.0,
            bypass: false,
            sample_rate: 48_000.0,
            attack_coeff: 0.0,
            release_coeff: 0.0,
            det_release: 0.0,
            hold_samples: 0,
            env: 0.0,
            gain: 1.0,
            hold_counter: 0,
        }
    }
}

impl Gate {
    /// A practical gate for a single voice mic.
    pub fn voice(threshold_db: f32) -> Self {
        Self {
            threshold_db,
            range_db: -60.0,
            ..Default::default()
        }
    }
}

impl Effect for Gate {
    fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.attack_coeff = time_coeff(self.attack_ms, sample_rate);
        self.release_coeff = time_coeff(self.release_ms, sample_rate);
        self.det_release = time_coeff(10.0, sample_rate); // 10ms detector release
        self.hold_samples = (self.hold_ms * 0.001 * sample_rate) as u32;
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass {
            return;
        }
        let threshold = db_to_gain(self.threshold_db);
        let closed_gain = db_to_gain(self.range_db);

        for s in block.iter_mut() {
            // Peak detector: instant attack, smoothed release.
            let rectified = s.abs();
            self.env = if rectified > self.env {
                rectified
            } else {
                rectified + self.det_release * (self.env - rectified)
            };

            // Open/closed decision with hold.
            let target = if self.env >= threshold {
                self.hold_counter = self.hold_samples;
                1.0
            } else if self.hold_counter > 0 {
                self.hold_counter -= 1;
                1.0
            } else {
                closed_gain
            };

            // Smooth the gain toward the target (attack when opening).
            let coeff = if target > self.gain {
                self.attack_coeff
            } else {
                self.release_coeff
            };
            self.gain = target + coeff * (self.gain - target);
            *s *= self.gain;
        }
    }

    fn reset(&mut self) {
        self.env = 0.0;
        self.gain = 1.0;
        self.hold_counter = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{rms, sine};

    const SR: f32 = 48_000.0;

    /// Steady-state gain (dB) the gate applies to a sine of a given amplitude.
    fn gain_db(gate: &mut Gate, amp: f32) -> f32 {
        let n = SR as usize; // 1 second to settle hold/release
        let input: Vec<f32> = sine(220.0, SR, n).iter().map(|s| s * amp).collect();
        let mut buf = input.clone();
        gate.process(&mut buf);
        let half = n / 2;
        super::super::gain_to_db(rms(&buf[half..]) / rms(&input[half..]))
    }

    #[test]
    fn default_is_transparent() {
        let mut g = Gate::default();
        g.prepare(SR);
        assert!(gain_db(&mut g, 0.01).abs() < 0.5); // even quiet passes
    }

    #[test]
    fn closes_on_quiet_opens_on_loud() {
        // Threshold -20 dBFS.
        let mut g = Gate::voice(-20.0);
        g.prepare(SR);
        // Quiet signal (~-40 dBFS peak) → strongly attenuated.
        assert!(gain_db(&mut g, 0.01) < -20.0);

        g.reset();
        // Loud signal (0 dBFS) → passes essentially untouched.
        assert!(gain_db(&mut g, 1.0).abs() < 0.5);
    }

    #[test]
    fn output_is_finite() {
        let mut g = Gate::voice(-30.0);
        g.prepare(SR);
        let mut buf = sine(220.0, SR, 4800);
        g.process(&mut buf);
        assert!(buf.iter().all(|s| s.is_finite()));
    }
}
