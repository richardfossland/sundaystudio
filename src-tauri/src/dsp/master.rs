//! The master chain (Phase 4.2b) — what processes the final mix before export.
//!
//! Signal order: a gentle final EQ → 3-band compressor (glue) → brick-wall
//! limiter (the ceiling). It's deliberately light; the voice chain (Phase 4.1)
//! does the heavy lifting per track, and the master just finishes the mix and
//! guarantees the peak ceiling.
//!
//! [`master_normalize`] is the two-pass loudness normaliser the platforms want:
//! measure the integrated loudness, apply the gain needed to hit the target, and
//! let the limiter catch the peaks that gain pushes up. Unlike the gain-only
//! [`normalize_clip_safe`](super::loudness::normalize_clip_safe), the limiter
//! lets us actually *reach* a loud target (-14 LUFS) without clipping.

use super::eq::ParametricEq;
use super::limiter::Limiter;
use super::loudness::{self, LoudnessError, LoudnessTarget, NormalizationReport};
use super::multiband::MultibandCompressor;
use super::Effect;

/// The bundled master processing chain.
#[derive(Debug, Clone, Default)]
pub struct MasterChain {
    pub eq: ParametricEq,
    pub multiband: MultibandCompressor,
    pub limiter: Limiter,
}

impl MasterChain {
    /// A gentle podcast master: voice-glue multiband + a -1 dBFS brick wall.
    /// The EQ stays transparent unless a preset (Phase 4.2c) shapes it.
    pub fn voice() -> Self {
        Self {
            eq: ParametricEq::default(),
            multiband: MultibandCompressor::voice(),
            limiter: Limiter::brickwall(-1.0),
        }
    }
}

impl Effect for MasterChain {
    fn prepare(&mut self, sample_rate: f32) {
        self.eq.prepare(sample_rate);
        self.multiband.prepare(sample_rate);
        self.limiter.prepare(sample_rate);
    }

    fn process(&mut self, block: &mut [f32]) {
        self.eq.process(block);
        self.multiband.process(block);
        self.limiter.process(block);
    }

    fn reset(&mut self) {
        self.eq.reset();
        self.multiband.reset();
        self.limiter.reset();
    }
}

/// Two-pass loudness normalisation through the master chain.
///
/// Order matters: tone and glue (EQ + multiband, which themselves change level
/// via filtering and auto-makeup) run first, *then* we measure and apply the
/// loudness gain toward the target, and the limiter runs last so it always has
/// the final word on the ceiling. The gain is uncapped — the limiter, set to the
/// target's true-peak ceiling, catches the peaks it raises, which is what lets a
/// quiet mix actually reach a loud target without clipping. Pass two re-measures
/// so the report tells the truth about what was achieved.
///
/// Note: the limiter introduces a few ms of look-ahead latency, so the buffer is
/// shifted by that much; over a whole programme it doesn't move the integrated
/// number meaningfully. Sample-accurate latency compensation belongs to the
/// export renderer (Phase 7).
pub fn master_normalize(
    samples: &mut [f32],
    channels: u32,
    rate: u32,
    target: &LoudnessTarget,
    chain: &mut MasterChain,
) -> Result<NormalizationReport, LoudnessError> {
    let before = loudness::measure(samples, channels, rate)?;

    if before.integrated_lufs.is_none() {
        return Ok(NormalizationReport {
            target_lufs: target.integrated_lufs,
            gain_applied_db: 0.0,
            after: before,
            before,
            gain_capped_by_peak: false,
            reached_target: false,
        });
    }

    chain.limiter.ceiling_db = target.true_peak_ceiling_dbtp;
    chain.prepare(rate as f32);

    // Tone + glue first — these shift level (EQ, makeup gain), so the loudness
    // gain has to be computed against the *glued* signal, not the raw input.
    chain.eq.process(samples);
    chain.multiband.process(samples);
    let glued = loudness::measure(samples, channels, rate)?;

    // Aim straight for the target; the limiter (last) keeps us clip-safe.
    let gain_db = glued
        .integrated_lufs
        .map(|l| target.integrated_lufs - l)
        .unwrap_or(0.0);
    let lin = 10.0_f32.powf(gain_db / 20.0);
    for s in samples.iter_mut() {
        *s *= lin;
    }
    chain.limiter.process(samples);

    let after = loudness::measure(samples, channels, rate)?;
    let reached_target = after
        .integrated_lufs
        .map(|l| (l - target.integrated_lufs).abs() <= 1.0)
        .unwrap_or(false);

    Ok(NormalizationReport {
        target_lufs: target.integrated_lufs,
        gain_applied_db: gain_db,
        before,
        after,
        // The limiter replaces the gain cap; loudness is reached via limiting.
        gain_capped_by_peak: false,
        reached_target,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::loudness::target_by_id;
    use crate::dsp::testutil::{peak, sine};

    const SR_F: f32 = 48_000.0;
    const SR: u32 = 48_000;

    #[test]
    fn transparent_default_chain_is_roughly_flat() {
        let mut m = MasterChain::default();
        m.prepare(SR_F);
        // Default: transparent EQ, unity multiband, limiter only catches >-1 dBFS.
        // A moderate tone passes essentially unchanged in level.
        let n = SR as usize / 2;
        let input: Vec<f32> = sine(1000.0, SR_F, n).iter().map(|s| s * 0.3).collect();
        let mut buf = input.clone();
        m.process(&mut buf);
        let half = n / 2;
        let rms_in = crate::dsp::testutil::rms(&input[half..]);
        let rms_out = crate::dsp::testutil::rms(&buf[half..]);
        let g = super::super::gain_to_db(rms_out / rms_in);
        assert!(g.abs() < 1.5, "default master altered level by {g} dB");
    }

    #[test]
    fn normalize_reaches_loud_target_without_clipping() {
        // A quiet sine normalised up to Spotify's -14 LUFS: the limiter must
        // hold the ceiling while we get close to target loudness.
        let mut buf = sine(300.0, SR_F, SR as usize * 6).iter().map(|s| s * 0.15).collect::<Vec<_>>();
        let target = target_by_id("spotify").unwrap(); // -14 LUFS, -1 dBTP
        let mut chain = MasterChain::voice();
        let report = master_normalize(&mut buf, 1, SR, &target, &mut chain).unwrap();

        // No clipping: every sample respects the ceiling (with a little inter-
        // sample slack), and certainly nothing over full scale.
        assert!(peak(&buf) <= 1.0, "clipped: peak {}", peak(&buf));
        // Loudness landed near the target.
        let achieved = report.after.integrated_lufs.unwrap();
        assert!(
            (achieved - target.integrated_lufs).abs() <= 1.5,
            "achieved {achieved} LUFS, target {}",
            target.integrated_lufs
        );
    }

    #[test]
    fn silence_normalizes_to_a_noop() {
        let mut buf = vec![0.0_f32; SR as usize];
        let target = target_by_id("apple-podcasts").unwrap();
        let mut chain = MasterChain::voice();
        let report = master_normalize(&mut buf, 1, SR, &target, &mut chain).unwrap();
        assert_eq!(report.gain_applied_db, 0.0);
        assert!(!report.reached_target);
    }
}
