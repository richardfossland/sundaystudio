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

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::compressor::Compressor;
use super::eq::{EqBand, EqBandType, ParametricEq};
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

/// A bundled mastering preset: a complete master chain paired with the platform
/// loudness target it's meant to be normalised to. One-click "make it sound
/// finished and ship-ready for X".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MasterPreset {
    /// Light glue + a presence lift, normalised to Spotify (-14 LUFS).
    ConversationPodcast,
    /// Broader dynamics, less compression, normalised to Apple Podcasts (-16).
    Sermon,
    /// Preserve dynamics, only light limiting, normalised to Spotify.
    MusicHeavy,
    /// Aggressive and bright — loud on purpose (use at your own risk).
    LoudAndBright,
}

impl MasterPreset {
    pub const ALL: [MasterPreset; 4] = [
        MasterPreset::ConversationPodcast,
        MasterPreset::Sermon,
        MasterPreset::MusicHeavy,
        MasterPreset::LoudAndBright,
    ];

    pub fn id(self) -> &'static str {
        match self {
            MasterPreset::ConversationPodcast => "conversation-podcast",
            MasterPreset::Sermon => "sermon",
            MasterPreset::MusicHeavy => "music-heavy",
            MasterPreset::LoudAndBright => "loud-and-bright",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            MasterPreset::ConversationPodcast => "Conversation Podcast",
            MasterPreset::Sermon => "Sermon",
            MasterPreset::MusicHeavy => "Music-heavy",
            MasterPreset::LoudAndBright => "Loud & Bright",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            MasterPreset::ConversationPodcast => {
                "Light glue and a presence lift; even, consistent talk. → Spotify -14 LUFS."
            }
            MasterPreset::Sermon => {
                "Keeps the dynamics of a live room; gentle control. → Apple Podcasts -16 LUFS."
            }
            MasterPreset::MusicHeavy => {
                "Preserves dynamics, only catches peaks. For music-forward shows. → Spotify -14."
            }
            MasterPreset::LoudAndBright => {
                "Aggressive and bright — loud on purpose. Signals amateur; use sparingly. → -14."
            }
        }
    }

    /// The platform loudness target this preset normalises to.
    pub fn target_id(self) -> &'static str {
        match self {
            MasterPreset::Sermon => "apple-podcasts",
            _ => "spotify",
        }
    }

    pub fn from_id(id: &str) -> Option<MasterPreset> {
        MasterPreset::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Materialise the preset into a configured (un-prepared) master chain.
    pub fn build(self) -> MasterChain {
        let limiter = Limiter::brickwall(-1.0);
        match self {
            MasterPreset::ConversationPodcast => MasterChain {
                eq: ParametricEq::from_bands(&[EqBand::bell(3000.0, 0.9, 1.5)]),
                multiband: MultibandCompressor::voice(),
                limiter,
            },
            MasterPreset::Sermon => {
                // Less compression: higher thresholds, lower ratios; slower
                // limiter release so the room breathes.
                let mut mb = MultibandCompressor::voice();
                mb.low = Compressor::voice(-18.0, 1.5);
                mb.mid = Compressor::voice(-16.0, 1.8);
                mb.high = Compressor::voice(-18.0, 1.5);
                let mut limiter = limiter;
                limiter.release_ms = 160.0; // slower recovery — let the room breathe
                MasterChain {
                    eq: ParametricEq::default(),
                    multiband: mb,
                    limiter,
                }
            }
            MasterPreset::MusicHeavy => {
                // Near-transparent glue; rely on the limiter for peaks only.
                let mut mb = MultibandCompressor::default();
                mb.low = Compressor::voice(-12.0, 1.3);
                mb.mid = Compressor::voice(-12.0, 1.3);
                mb.high = Compressor::voice(-12.0, 1.3);
                MasterChain {
                    eq: ParametricEq::default(),
                    multiband: mb,
                    limiter,
                }
            }
            MasterPreset::LoudAndBright => {
                let mut mb = MultibandCompressor::voice();
                mb.low = Compressor::voice(-26.0, 3.5);
                mb.mid = Compressor::voice(-24.0, 4.0);
                mb.high = Compressor::voice(-26.0, 3.5);
                MasterChain {
                    eq: ParametricEq::from_bands(&[
                        EqBand::bell(4000.0, 0.9, 3.0),
                        EqBand {
                            enabled: true,
                            band_type: EqBandType::HighShelf,
                            freq: 9000.0,
                            q: 0.707,
                            gain_db: 3.0,
                        },
                    ]),
                    multiband: mb,
                    limiter,
                }
            }
        }
    }
}

/// Mastering-preset metadata for the UI picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/MasterPresetInfo.ts")]
pub struct MasterPresetInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    /// The loudness target id this preset normalises to (see `LoudnessTarget`).
    pub target_id: String,
}

/// All mastering presets, for `dsp_master_presets`.
pub fn master_preset_infos() -> Vec<MasterPresetInfo> {
    MasterPreset::ALL
        .into_iter()
        .map(|p| MasterPresetInfo {
            id: p.id().to_string(),
            label: p.label().to_string(),
            description: p.description().to_string(),
            target_id: p.target_id().to_string(),
        })
        .collect()
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
    fn master_presets_are_well_formed_and_build() {
        let infos = master_preset_infos();
        assert_eq!(infos.len(), 4);
        assert!(MasterPreset::from_id("sermon").is_some());
        assert!(MasterPreset::from_id("nope").is_none());
        for info in &infos {
            // Every preset's declared target must resolve to a real target.
            assert!(
                target_by_id(&info.target_id).is_some(),
                "{} -> unknown target {}",
                info.id,
                info.target_id
            );
        }
        // Each preset builds a chain that stays finite on a real tone.
        for p in MasterPreset::ALL {
            let mut chain = p.build();
            chain.prepare(SR_F);
            let mut buf = sine(1000.0, SR_F, 4800);
            chain.process(&mut buf);
            assert!(
                buf.iter().all(|s| s.is_finite()),
                "{} produced non-finite output",
                p.label()
            );
        }
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
