//! Brick-wall look-ahead limiter (Phase 4.2b) — the last thing the master sees.
//!
//! A limiter is a compressor with an infinite ratio and a hard ceiling: nothing
//! gets out louder than the ceiling. On the master it does two jobs — it catches
//! the stray peaks that loudness normalisation's gain would push over 0 dBFS, and
//! it lets us reach a loud platform target (-14 LUFS) without clipping.
//!
//! Look-ahead is what makes it transparent: we delay the audio by a few
//! milliseconds and *peek* at what's coming, so the gain is already pulled down
//! by the time a transient arrives — no overshoot, no clipping on the attack.
//! We track the minimum allowed gain across the look-ahead window with a
//! monotonic min-deque (O(1) per sample), so the envelope can never sit above
//! what an upcoming peak demands. Only the *release* (recovery toward unity) is
//! smoothed, which is what keeps it from pumping.
//!
//! This limits on the **sample** peak with the ceiling set a hair below the
//! true-peak target, leaving headroom for inter-sample peaks; the master's
//! loudness re-measure (Phase 4.2a) verifies the true-peak result. A full 4×
//! oversampling true-peak limiter is a later refinement if measurements demand.

use std::collections::VecDeque;

use super::{db_to_gain, Effect};

/// One entry in the look-ahead min-deque: the sample index a gain was computed
/// at, and that gain. The deque keeps gains monotonically increasing so the
/// front is always the window minimum.
#[derive(Debug, Clone, Copy)]
struct GainAt {
    index: u64,
    gain: f32,
}

#[derive(Debug, Clone)]
pub struct Limiter {
    /// Output ceiling in dBFS (negative). Peaks are held at or below this.
    pub ceiling_db: f32,
    /// Look-ahead time in milliseconds (typical 1–5 ms).
    pub lookahead_ms: f32,
    /// Release time in milliseconds — how fast gain recovers after a peak.
    pub release_ms: f32,
    pub bypass: bool,

    sample_rate: f32,
    ceiling_lin: f32,
    lookahead: usize,
    release_coeff: f32,

    // Delay line for the look-ahead (the signal is output `lookahead` samples
    // late so the gain reduction lands before the peak does).
    delay: Vec<f32>,
    widx: usize,
    // Monotonic-increasing min-deque of upcoming gain targets.
    window: VecDeque<GainAt>,
    // Running sample counter, used as the window index.
    n: u64,
    // Smoothed gain envelope (≤ window minimum, ≤ 1).
    env: f32,
}

impl Default for Limiter {
    fn default() -> Self {
        Self {
            ceiling_db: -1.0,
            lookahead_ms: 3.0,
            release_ms: 80.0,
            bypass: false,
            sample_rate: 48_000.0,
            ceiling_lin: db_to_gain(-1.0),
            lookahead: 1,
            release_coeff: 0.0,
            delay: vec![0.0; 1],
            widx: 0,
            window: VecDeque::new(),
            n: 0,
            env: 1.0,
        }
    }
}

impl Limiter {
    /// A master-bus brick-wall limiter at the given ceiling (dBFS).
    pub fn brickwall(ceiling_db: f32) -> Self {
        Self {
            ceiling_db,
            ..Default::default()
        }
    }

    /// Samples of latency this limiter introduces (the look-ahead delay).
    pub fn latency_samples(&self) -> usize {
        self.lookahead
    }

    /// Gain that would keep `sample` at or below the ceiling (≤ 1.0).
    fn target_gain(&self, sample: f32) -> f32 {
        let peak = sample.abs();
        if peak > self.ceiling_lin {
            self.ceiling_lin / peak
        } else {
            1.0
        }
    }
}

impl Effect for Limiter {
    fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.ceiling_lin = db_to_gain(self.ceiling_db);
        self.lookahead = ((self.lookahead_ms * 0.001 * sample_rate) as usize).max(1);
        // Per-sample release factor toward unity (time to ~63% recovery).
        self.release_coeff = if self.release_ms > 0.0 {
            1.0 - (-1.0 / (self.release_ms * 0.001 * sample_rate)).exp()
        } else {
            1.0
        };
        self.reset();
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass {
            return;
        }
        for x in block.iter_mut() {
            let input = *x;
            let g = self.target_gain(input);

            // Push onto the monotonic min-deque, dropping any tail gains this
            // one undercuts (they can never be the window minimum again).
            while self.window.back().is_some_and(|b| b.gain >= g) {
                self.window.pop_back();
            }
            self.window.push_back(GainAt { index: self.n, gain: g });
            // Drop entries that have fallen out of the look-ahead window.
            let oldest = self.n.saturating_sub(self.lookahead as u64);
            while self.window.front().is_some_and(|f| f.index < oldest) {
                self.window.pop_front();
            }
            let window_min = self.window.front().map(|f| f.gain).unwrap_or(1.0);

            // Output the delayed sample; store the new one in its place.
            let delayed = self.delay[self.widx];
            self.delay[self.widx] = input;
            self.widx = (self.widx + 1) % self.lookahead;

            // Release toward unity, but never above what the window demands
            // (instant attack, smoothed release — always clip-safe).
            self.env = (self.env + self.release_coeff * (1.0 - self.env)).min(window_min);
            *x = delayed * self.env;

            self.n += 1;
        }
    }

    fn reset(&mut self) {
        self.delay.clear();
        self.delay.resize(self.lookahead, 0.0);
        self.widx = 0;
        self.window.clear();
        self.window.reserve(self.lookahead + 1);
        self.n = 0;
        self.env = 1.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::{peak, sine};

    const SR: f32 = 48_000.0;

    #[test]
    fn holds_output_below_ceiling() {
        let mut lim = Limiter::brickwall(-1.0);
        lim.prepare(SR);
        // A full-scale sine: every peak must be pulled under the ceiling.
        let mut buf = sine(220.0, SR, 24_000);
        lim.process(&mut buf);
        let ceiling = db_to_gain(-1.0);
        // Skip the initial look-ahead latency where the delay line is priming.
        let settled = &buf[lim.latency_samples()..];
        assert!(
            peak(settled) <= ceiling + 1e-3,
            "peak {} exceeds ceiling {}",
            peak(settled),
            ceiling
        );
    }

    #[test]
    fn passes_quiet_signal_essentially_untouched() {
        let mut lim = Limiter::brickwall(-1.0);
        lim.prepare(SR);
        // Well below the ceiling → gain stays at unity, only delayed.
        let input = sine(220.0, SR, 12_000).iter().map(|s| s * 0.3).collect::<Vec<_>>();
        let mut buf = input.clone();
        lim.process(&mut buf);
        let l = lim.latency_samples();
        // Compare aligned regions past the latency: output ≈ input, delayed.
        for i in (l + 100)..(input.len() - 1) {
            assert!((buf[i] - input[i - l]).abs() < 1e-4, "drift at {i}");
        }
    }

    #[test]
    fn no_overshoot_on_a_transient() {
        let mut lim = Limiter::brickwall(-1.0);
        lim.prepare(SR);
        // Silence, then a sudden full-scale burst — look-ahead must catch it
        // with no sample over the ceiling.
        let mut buf = vec![0.0_f32; 6_000];
        for s in buf.iter_mut().skip(3_000) {
            *s = 0.95;
        }
        lim.process(&mut buf);
        let ceiling = db_to_gain(-1.0);
        assert!(peak(&buf) <= ceiling + 1e-3, "transient overshoot: {}", peak(&buf));
    }
}
