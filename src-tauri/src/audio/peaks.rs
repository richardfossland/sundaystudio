//! Waveform peak computation + on-disk cache (Phase 3.1).
//!
//! The timeline renders each take's audio as a waveform. Decoding a 90-minute
//! WAV on every zoom/scroll would be ruinous, so we reduce a take's per-track
//! mono samples to a bounded array of normalised peak magnitudes once, cache it
//! to `<scast>/cache/peaks/<take_id>/<track_id>.json`, and let the canvas
//! downsample further per zoom level.
//!
//! Peaks are absolute (max |sample| per bucket, clamped to 1.0), not
//! auto-normalised to the file's loudest point — so a quiet recording reads as a
//! quiet waveform, the way a real editor shows it. The bucket size adapts to the
//! file length to keep the array (and the IPC payload) under `TARGET_MAX_PEAKS`
//! regardless of duration; true multi-resolution pyramids are a later refinement.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};
use crate::export::render::read_wav_mono;

/// Upper bound on the number of peaks returned for any take, however long.
const TARGET_MAX_PEAKS: usize = 20_000;
/// Floor on the bucket size, so short clips still get a smooth (not blocky) curve.
const MIN_SAMPLES_PER_PEAK: usize = 256;

/// A take track's precomputed waveform overview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/WaveformPeaks.ts")]
pub struct WaveformPeaks {
    pub take_id: String,
    pub source_track_id: String,
    pub sample_rate: u32,
    pub duration_ms: f64,
    /// Audio samples summarised by each entry in `peaks`.
    pub samples_per_peak: u32,
    /// Normalised peak magnitudes 0..1, one per bucket of `samples_per_peak`.
    pub peaks: Vec<f32>,
}

/// Reduce mono samples to at most `TARGET_MAX_PEAKS` absolute peak magnitudes.
/// Returns the chosen bucket size alongside the peaks so callers can map a peak
/// index back to a sample offset (and thus a time).
pub fn compute_peaks(samples: &[f32]) -> (u32, Vec<f32>) {
    if samples.is_empty() {
        return (MIN_SAMPLES_PER_PEAK as u32, Vec::new());
    }
    // Ceil-divide so we never exceed the target, then floor at the minimum.
    let spp = samples
        .len()
        .div_ceil(TARGET_MAX_PEAKS)
        .max(MIN_SAMPLES_PER_PEAK);
    let mut peaks = Vec::with_capacity(samples.len() / spp + 1);
    for chunk in samples.chunks(spp) {
        let m = chunk.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
        peaks.push(m.min(1.0));
    }
    (spp as u32, peaks)
}

/// Where a track's cached peaks live inside the project folder.
fn cache_path(scast_dir: &Path, take_id: &str, source_track_id: &str) -> PathBuf {
    scast_dir
        .join("cache")
        .join("peaks")
        .join(take_id)
        .join(format!("{source_track_id}.json"))
}

/// Return a take track's peaks, computing and caching them on the first request.
/// `take_dir` is `<scast>/takes/<take_id>`. Pure file IO — call from a blocking
/// task, not the async runtime.
pub fn load_or_compute(
    scast_dir: &Path,
    take_dir: &Path,
    take_id: &str,
    source_track_id: &str,
) -> AppResult<WaveformPeaks> {
    let cache = cache_path(scast_dir, take_id, source_track_id);
    if let Ok(bytes) = fs::read(&cache) {
        if let Ok(cached) = serde_json::from_slice::<WaveformPeaks>(&bytes) {
            return Ok(cached);
        }
        // Corrupt cache: fall through and recompute.
    }

    let wav = take_dir.join(format!("{source_track_id}.wav"));
    if !wav.exists() {
        return Err(AppError::NotFound {
            entity: "take audio",
            id: source_track_id.to_string(),
        });
    }
    let (samples, sample_rate) = read_wav_mono(&wav).map_err(AppError::Audio)?;
    let duration_ms = samples.len() as f64 / sample_rate.max(1) as f64 * 1000.0;
    let (samples_per_peak, peaks) = compute_peaks(&samples);

    let result = WaveformPeaks {
        take_id: take_id.to_string(),
        source_track_id: source_track_id.to_string(),
        sample_rate,
        duration_ms,
        samples_per_peak,
        peaks,
    };

    if let Some(parent) = cache.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&cache, serde_json::to_vec(&result)?);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_no_peaks() {
        let (spp, peaks) = compute_peaks(&[]);
        assert_eq!(spp, MIN_SAMPLES_PER_PEAK as u32);
        assert!(peaks.is_empty());
    }

    #[test]
    fn short_clip_uses_minimum_bucket() {
        let samples = vec![0.5_f32; 1000];
        let (spp, peaks) = compute_peaks(&samples);
        assert_eq!(spp, MIN_SAMPLES_PER_PEAK as u32);
        // 1000 / 256 → 4 buckets (ceil).
        assert_eq!(peaks.len(), 4);
        // Constant 0.5 input → every bucket peaks at 0.5.
        assert!(peaks.iter().all(|&p| (p - 0.5).abs() < 1e-6));
    }

    #[test]
    fn long_input_stays_under_target_and_captures_peak() {
        // 10M samples → bucketed so the array never exceeds TARGET_MAX_PEAKS.
        let mut samples = vec![0.1_f32; 10_000_000];
        samples[5_000_000] = 0.9; // a transient that must survive bucketing
        let (spp, peaks) = compute_peaks(&samples);
        assert!(peaks.len() <= TARGET_MAX_PEAKS, "got {}", peaks.len());
        assert!(spp as usize >= MIN_SAMPLES_PER_PEAK);
        // The bucket containing the transient holds its magnitude.
        let hit = peaks.iter().any(|&p| (p - 0.9).abs() < 1e-6);
        assert!(hit, "transient was lost in bucketing");
    }

    #[test]
    fn peaks_are_clamped_to_unit() {
        let samples = vec![-2.0_f32, 2.0, -3.0];
        let (_, peaks) = compute_peaks(&samples);
        assert!(peaks.iter().all(|&p| p <= 1.0));
    }
}
