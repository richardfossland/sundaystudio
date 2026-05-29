//! Loudness measurement and platform normalisation (Phase 4.2).
//!
//! Podcast platforms don't play your file at the level you exported it — they
//! normalise to a target *integrated loudness* measured in LUFS (Loudness Units
//! relative to Full Scale, per EBU R128 / ITU-R BS.1770). Master too quiet and
//! the platform turns you up (and the noise with you); master too loud and it
//! turns you down (wasting the headroom you fought for). The job of mastering is
//! to hit the target on purpose.
//!
//! This module wraps the [`ebur128`] crate — a pure-Rust port of `libebur128`,
//! the same measurement the reference tools use — so our numbers match what
//! Spotify, Apple and `ffmpeg -af ebur128` report. It gives us:
//!   - **integrated** loudness (the whole show, gated — what platforms target)
//!   - **short-term** (last 3 s) and **momentary** (last 400 ms) for live meters
//!   - **loudness range** (LRA, the macro-dynamics in LU)
//!   - **true peak** (inter-sample peak, dBTP) — the ceiling that matters once
//!     a lossy codec reconstructs the waveform.
//!
//! Two kinds of consumer: [`measure`] does a one-shot pass over a finished
//! buffer (for analysis and two-pass normalisation), and [`LoudnessMeter`]
//! streams blocks for the live UI meters that arrive with playback.
//!
//! Anything ebur128 reports as `-inf`/silence (a quiet enough buffer never
//! crosses the absolute gate) becomes `None` here — `serde_json` cannot encode
//! a non-finite float, so the IPC contract is `number | null`, not `NaN`.

use ebur128::{EbuR128, Mode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Errors surfaced by the loudness layer. Re-exported so callers can convert.
pub use ebur128::Error as LoudnessError;

/// Convert a non-negative linear amplitude to dBFS, or `None` for silence /
/// non-finite input (so it serialises cleanly across IPC).
fn lin_to_db(x: f64) -> Option<f32> {
    if x.is_finite() && x > 0.0 {
        Some((20.0 * x.log10()) as f32)
    } else {
        None
    }
}

/// Pass a loudness/LU value through only if it is finite (ebur128 returns
/// `-inf` when a buffer never crosses the absolute gate, i.e. it is silent).
fn finite(x: f64) -> Option<f32> {
    if x.is_finite() {
        Some(x as f32)
    } else {
        None
    }
}

/// A loudness snapshot. Every field is `None` when undefined (silence, or a
/// window that hasn't filled yet) so it crosses IPC as `number | null`.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LoudnessMeasurement.ts")]
pub struct LoudnessMeasurement {
    /// Integrated (gated, whole-program) loudness — the platform target metric.
    pub integrated_lufs: Option<f32>,
    /// Short-term loudness over the trailing 3 seconds.
    pub short_term_lufs: Option<f32>,
    /// Momentary loudness over the trailing 400 ms.
    pub momentary_lufs: Option<f32>,
    /// Loudness range (LRA) in LU — the program's macro-dynamics.
    pub loudness_range_lu: Option<f32>,
    /// True (inter-sample) peak in dBTP, across all channels.
    pub true_peak_dbtp: Option<f32>,
    /// Sample peak in dBFS, across all channels.
    pub sample_peak_dbfs: Option<f32>,
}

/// The processing mode used everywhere here: integrated + range + both peak
/// kinds. `LRA` implies short-term implies momentary, and `TRUE_PEAK`/`I` both
/// imply momentary, so this single union unlocks every getter we read.
fn full_mode() -> Mode {
    Mode::I | Mode::LRA | Mode::TRUE_PEAK | Mode::SAMPLE_PEAK
}

/// Read every metric off a prepared analyser. Peaks are reduced across channels
/// (the worst channel is the one that clips).
fn snapshot(ebu: &EbuR128, channels: u32) -> LoudnessMeasurement {
    let mut true_peak = 0.0_f64;
    let mut sample_peak = 0.0_f64;
    for ch in 0..channels {
        true_peak = true_peak.max(ebu.true_peak(ch).unwrap_or(0.0));
        sample_peak = sample_peak.max(ebu.sample_peak(ch).unwrap_or(0.0));
    }
    LoudnessMeasurement {
        integrated_lufs: ebu.loudness_global().ok().and_then(finite),
        short_term_lufs: ebu.loudness_shortterm().ok().and_then(finite),
        momentary_lufs: ebu.loudness_momentary().ok().and_then(finite),
        loudness_range_lu: ebu.loudness_range().ok().and_then(finite),
        true_peak_dbtp: lin_to_db(true_peak),
        sample_peak_dbfs: lin_to_db(sample_peak),
    }
}

/// One-shot measurement of an interleaved buffer (`channels`-interleaved f32).
/// Use for analysing a finished take or the pre/post passes of normalisation.
pub fn measure(
    samples: &[f32],
    channels: u32,
    rate: u32,
) -> Result<LoudnessMeasurement, LoudnessError> {
    let mut ebu = EbuR128::new(channels, rate, full_mode())?;
    ebu.add_frames_f32(samples)?;
    Ok(snapshot(&ebu, channels))
}

/// A platform's normalisation target: an integrated-loudness goal and the true-
/// peak ceiling it expects you to stay under.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LoudnessTarget.ts")]
pub struct LoudnessTarget {
    pub id: String,
    pub label: String,
    /// Target integrated loudness (LUFS). Negative.
    pub integrated_lufs: f32,
    /// Maximum allowed true peak (dBTP). Negative.
    pub true_peak_ceiling_dbtp: f32,
    pub description: String,
}

/// `const`-friendly twin of [`LoudnessTarget`] (string literals, no `String`),
/// so the catalogue can live in a `const` array.
struct StaticTarget {
    id: &'static str,
    label: &'static str,
    integrated_lufs: f32,
    true_peak_ceiling_dbtp: f32,
    description: &'static str,
}

impl StaticTarget {
    const fn new(
        id: &'static str,
        label: &'static str,
        integrated_lufs: f32,
        true_peak_ceiling_dbtp: f32,
        description: &'static str,
    ) -> Self {
        Self {
            id,
            label,
            integrated_lufs,
            true_peak_ceiling_dbtp,
            description,
        }
    }

    fn to_target(&self) -> LoudnessTarget {
        LoudnessTarget {
            id: self.id.to_string(),
            label: self.label.to_string(),
            integrated_lufs: self.integrated_lufs,
            true_peak_ceiling_dbtp: self.true_peak_ceiling_dbtp,
            description: self.description.to_string(),
        }
    }
}

/// The platform presets we ship. Spotify/YouTube target -14 LUFS, Apple -16,
/// EBU broadcast -23; all cap true peak at -1 dBTP (the standard lossy-codec
/// safety margin).
const TARGETS: [StaticTarget; 4] = [
    StaticTarget::new(
        "spotify",
        "Spotify",
        -14.0,
        -1.0,
        "Spotify & most music-forward platforms normalise to -14 LUFS.",
    ),
    StaticTarget::new(
        "apple-podcasts",
        "Apple Podcasts",
        -16.0,
        -1.0,
        "Apple Podcasts targets -16 LUFS — a touch quieter, speech-friendly.",
    ),
    StaticTarget::new(
        "youtube",
        "YouTube",
        -14.0,
        -1.0,
        "YouTube normalises uploads to around -14 LUFS.",
    ),
    StaticTarget::new(
        "broadcast-eu",
        "Broadcast (EBU R128)",
        -23.0,
        -1.0,
        "European broadcast standard: -23 LUFS, -1 dBTP. Quietest target.",
    ),
];

/// The platform loudness targets, for the UI's "Match to platform" picker.
pub fn loudness_targets() -> Vec<LoudnessTarget> {
    TARGETS.iter().map(StaticTarget::to_target).collect()
}

/// Look a target up by id.
pub fn target_by_id(id: &str) -> Option<LoudnessTarget> {
    TARGETS
        .iter()
        .find(|t| t.id == id)
        .map(StaticTarget::to_target)
}

/// The result of a normalisation pass: how much gain we applied, the before/
/// after measurements, and whether we could hit the target without clipping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/NormalizationReport.ts")]
pub struct NormalizationReport {
    pub target_lufs: f32,
    pub gain_applied_db: f32,
    pub before: LoudnessMeasurement,
    pub after: LoudnessMeasurement,
    /// True when the target loudness was unreachable without exceeding the true-
    /// peak ceiling, so gain was capped to stay clip-safe. A brick-wall limiter
    /// (Phase 4.2b) is what lets you push past this honestly.
    pub gain_capped_by_peak: bool,
    /// True when the achieved integrated loudness is within 0.5 LU of target.
    pub reached_target: bool,
}

/// Clip-safe loudness normalisation: measure, then apply a single gain to hit
/// the target integrated loudness — but never more than the true-peak ceiling
/// allows. With gain alone (no limiter) this guarantees no new clipping; if the
/// peaks won't permit reaching the target, we get as loud as safely possible and
/// flag it via `gain_capped_by_peak`. The limiter in Phase 4.2b removes that cap.
pub fn normalize_clip_safe(
    samples: &mut [f32],
    channels: u32,
    rate: u32,
    target: &LoudnessTarget,
) -> Result<NormalizationReport, LoudnessError> {
    let before = measure(samples, channels, rate)?;

    // Silence (no integrated reading) — nothing to normalise.
    let Some(integrated) = before.integrated_lufs else {
        return Ok(NormalizationReport {
            target_lufs: target.integrated_lufs,
            gain_applied_db: 0.0,
            after: before,
            before,
            gain_capped_by_peak: false,
            reached_target: false,
        });
    };

    let desired_gain = target.integrated_lufs - integrated;
    // Headroom before we'd breach the true-peak ceiling. If we somehow have no
    // peak reading, don't constrain (desired wins).
    let peak_headroom = before
        .true_peak_dbtp
        .map(|tp| target.true_peak_ceiling_dbtp - tp)
        .unwrap_or(f32::INFINITY);

    let gain_applied = desired_gain.min(peak_headroom);
    let gain_capped_by_peak = desired_gain > peak_headroom + 1e-4;

    let lin = 10.0_f32.powf(gain_applied / 20.0);
    for s in samples.iter_mut() {
        *s *= lin;
    }

    let after = measure(samples, channels, rate)?;
    let reached_target = after
        .integrated_lufs
        .map(|l| (l - target.integrated_lufs).abs() <= 0.5)
        .unwrap_or(false);

    Ok(NormalizationReport {
        target_lufs: target.integrated_lufs,
        gain_applied_db: gain_applied,
        before,
        after,
        gain_capped_by_peak,
        reached_target,
    })
}

/// Streaming loudness meter for the live UI. Feed it interleaved blocks as they
/// play; read [`LoudnessMeter::snapshot`] for the meters. Momentary/short-term
/// track the trailing windows; integrated accumulates over everything fed since
/// the last [`reset`](LoudnessMeter::reset).
pub struct LoudnessMeter {
    ebu: EbuR128,
    channels: u32,
}

impl LoudnessMeter {
    pub fn new(channels: u32, rate: u32) -> Result<Self, LoudnessError> {
        Ok(Self {
            ebu: EbuR128::new(channels, rate, full_mode())?,
            channels,
        })
    }

    /// Feed one interleaved block. Cheap enough for the UI cadence; this is NOT
    /// the real-time audio thread (it allocates internally) — drive it from the
    /// playback/metering side, never the cpal callback.
    pub fn push(&mut self, interleaved: &[f32]) -> Result<(), LoudnessError> {
        self.ebu.add_frames_f32(interleaved)
    }

    pub fn snapshot(&self) -> LoudnessMeasurement {
        snapshot(&self.ebu, self.channels)
    }

    pub fn reset(&mut self) {
        self.ebu.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    const SR: u32 = 48_000;

    /// `secs` of a mono sine at `freq`, amplitude `amp`.
    fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
        let n = (SR as f32 * secs) as usize;
        (0..n)
            .map(|i| amp * (freq * TAU * i as f32 / SR as f32).sin())
            .collect()
    }

    #[test]
    fn louder_signal_measures_higher_by_the_same_delta() {
        // A robust, cross-platform property: +6 dB amplitude => +6 LUFS.
        let quiet = measure(&sine(1000.0, 0.25, 4.0), 1, SR).unwrap();
        let loud = measure(&sine(1000.0, 0.5, 4.0), 1, SR).unwrap();
        let q = quiet.integrated_lufs.unwrap();
        let l = loud.integrated_lufs.unwrap();
        assert!(
            (l - q - 6.0206).abs() < 0.2,
            "expected ~+6 LUFS for +6 dB, got {q} -> {l}"
        );
    }

    #[test]
    fn full_scale_sine_true_peak_is_near_zero_dbfs() {
        let m = measure(&sine(997.0, 1.0, 2.0), 1, SR).unwrap();
        let tp = m.true_peak_dbtp.unwrap();
        // Inter-sample peak of a near-full-scale sine sits around 0 dBTP.
        assert!((-0.5..=1.0).contains(&tp), "true peak {tp} dBTP out of range");
        let sp = m.sample_peak_dbfs.unwrap();
        assert!(sp <= 0.1 && sp > -0.5, "sample peak {sp} dBFS out of range");
    }

    #[test]
    fn silence_reads_as_none_not_nan() {
        let m = measure(&vec![0.0_f32; SR as usize], 1, SR).unwrap();
        assert_eq!(m.integrated_lufs, None);
        assert_eq!(m.true_peak_dbtp, None);
        // And it must serialise (no NaN/inf reaches serde_json).
        assert!(serde_json::to_string(&m).is_ok());
    }

    #[test]
    fn targets_are_well_formed() {
        let targets = loudness_targets();
        assert_eq!(targets.len(), 4);
        assert!(target_by_id("spotify").is_some());
        assert!(target_by_id("nope").is_none());
        for t in targets {
            assert!(t.integrated_lufs < 0.0 && t.integrated_lufs >= -30.0);
            assert!(t.true_peak_ceiling_dbtp <= 0.0);
        }
    }

    #[test]
    fn normalize_lands_on_target_when_peaks_allow() {
        // A quiet sine has plenty of peak headroom, so gain alone hits target.
        let mut buf = sine(1000.0, 0.2, 6.0);
        let target = target_by_id("apple-podcasts").unwrap();
        let report = normalize_clip_safe(&mut buf, 1, SR, &target).unwrap();
        assert!(!report.gain_capped_by_peak);
        assert!(report.reached_target, "after = {:?}", report.after);
        let achieved = report.after.integrated_lufs.unwrap();
        assert!((achieved - target.integrated_lufs).abs() < 0.5);
    }

    #[test]
    fn normalize_stays_clip_safe_and_flags_cap() {
        // A high-crest-factor signal: a quiet sine (low integrated loudness)
        // with periodic full-scale spikes (no peak headroom). Reaching a louder
        // target would need positive gain the peaks won't allow, so it must be
        // capped to stay clip-safe.
        let mut buf = sine(1000.0, 0.06, 6.0);
        for i in (0..buf.len()).step_by(SR as usize / 2) {
            buf[i] = 1.0; // a full-scale spike twice a second
        }
        let target = target_by_id("spotify").unwrap(); // -14 LUFS, -1 dBTP
        let report = normalize_clip_safe(&mut buf, 1, SR, &target).unwrap();
        assert!(report.gain_capped_by_peak, "report = {report:?}");
        // Never exceed the ceiling: the cap keeps true peak at/under it.
        let tp_after = report.after.true_peak_dbtp.unwrap();
        assert!(tp_after <= target.true_peak_ceiling_dbtp + 0.3, "tp {tp_after}");
        assert!(buf.iter().all(|s| s.abs() <= 1.0001));
    }
}
