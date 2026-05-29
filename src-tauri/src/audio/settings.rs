//! Audio settings: device selection, sample rate, buffer size — and the
//! round-trip latency that those choices imply.
//!
//! Phase 1.1 stores settings at the app level (a single JSON file in the app
//! config dir). Phase 2.1 moves the *selection* into the project so reopening a
//! project restores its interface; this app-level file then holds only the
//! last-used defaults. The persistence and latency logic here are pure and
//! unit-tested; the Tauri commands just resolve the config path and delegate.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};

/// Sample rates we offer (Hz). Mirrors `devices::STANDARD_RATES`.
pub const SAMPLE_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];
/// Buffer sizes we offer (frames). Smaller = lower latency, higher CPU/risk.
pub const BUFFER_SIZES: [u32; 4] = [64, 128, 256, 512];

/// Persisted audio configuration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/AudioSettings.ts")]
pub struct AudioSettings {
    /// Selected input device name, or None to follow the system default.
    pub input_device: Option<String>,
    /// Selected output device name, or None to follow the system default.
    pub output_device: Option<String>,
    /// Project/engine sample rate in Hz.
    pub sample_rate: u32,
    /// I/O buffer size in frames.
    pub buffer_size: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        // 48 kHz / 256 frames: the safe, broadly-compatible podcast default.
        Self {
            input_device: None,
            output_device: None,
            sample_rate: 48_000,
            buffer_size: 256,
        }
    }
}

impl AudioSettings {
    /// Reject nonsensical combinations early (the UI only offers valid ones,
    /// but a hand-edited file or a future caller might not).
    pub fn validate(&self) -> AppResult<()> {
        if !SAMPLE_RATES.contains(&self.sample_rate) {
            return Err(AppError::Validation(format!(
                "unsupported sample rate: {}",
                self.sample_rate
            )));
        }
        if !BUFFER_SIZES.contains(&self.buffer_size) {
            return Err(AppError::Validation(format!(
                "unsupported buffer size: {}",
                self.buffer_size
            )));
        }
        Ok(())
    }
}

/// Latency zone for UI colour-coding (plan 1.3: green < 10ms, yellow 10–20,
/// red > 20).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LatencyZone.ts")]
#[serde(rename_all = "lowercase")]
pub enum LatencyZone {
    Green,
    Yellow,
    Red,
}

/// Estimated round-trip monitoring latency for a sample-rate / buffer choice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/LatencyEstimate.ts")]
pub struct LatencyEstimate {
    /// Round-trip estimate in milliseconds.
    pub ms: f32,
    pub zone: LatencyZone,
}

/// Round-trip latency estimate: input buffer + output buffer + a small fixed
/// driver/safety allowance. Real measured latency (queried from the live
/// stream) replaces this estimate in Phase 1.3; this is the a-priori figure the
/// settings screen shows before a stream is open.
pub fn roundtrip_latency_ms(sample_rate: u32, buffer_size: u32) -> f32 {
    const DRIVER_OVERHEAD_MS: f32 = 2.0;
    let one_buffer_ms = (buffer_size as f32 / sample_rate as f32) * 1000.0;
    one_buffer_ms * 2.0 + DRIVER_OVERHEAD_MS
}

/// Classify a latency in ms into a UI zone.
pub fn latency_zone(ms: f32) -> LatencyZone {
    if ms < 10.0 {
        LatencyZone::Green
    } else if ms <= 20.0 {
        LatencyZone::Yellow
    } else {
        LatencyZone::Red
    }
}

/// Full estimate for a settings choice.
pub fn estimate_latency(sample_rate: u32, buffer_size: u32) -> LatencyEstimate {
    let ms = roundtrip_latency_ms(sample_rate, buffer_size);
    LatencyEstimate {
        ms,
        zone: latency_zone(ms),
    }
}

/// The settings file inside a given config directory.
pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("audio.json")
}

/// Load settings from `path`. A missing file yields defaults (first run); a
/// corrupt file also yields defaults rather than failing the whole app —
/// audio config is recoverable, not precious.
pub fn load(path: &Path) -> AudioSettings {
    let Ok(bytes) = std::fs::read(path) else {
        return AudioSettings::default();
    };
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Persist settings to `path`, creating the parent directory if needed.
pub fn save(path: &Path, settings: &AudioSettings) -> AppResult<()> {
    settings.validate()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_vec_pretty(settings)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_safe() {
        let s = AudioSettings::default();
        assert_eq!(s.sample_rate, 48_000);
        assert_eq!(s.buffer_size, 256);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn validate_rejects_bad_values() {
        let bad_rate = AudioSettings {
            sample_rate: 12_345,
            ..Default::default()
        };
        assert!(bad_rate.validate().is_err());

        let bad_buffer = AudioSettings {
            buffer_size: 1000,
            ..Default::default()
        };
        assert!(bad_buffer.validate().is_err());
    }

    #[test]
    fn latency_scales_with_buffer_and_classifies() {
        // 256 frames @ 48k ≈ 5.33ms per buffer → ~12.7ms round-trip (yellow).
        let est = estimate_latency(48_000, 256);
        assert!((est.ms - 12.67).abs() < 0.1, "got {}", est.ms);
        assert_eq!(est.zone, LatencyZone::Yellow);

        // 64 frames @ 48k ≈ 1.33ms per buffer → ~4.67ms (green).
        assert_eq!(estimate_latency(48_000, 64).zone, LatencyZone::Green);

        // 512 frames @ 44.1k → ~25.2ms (red).
        assert_eq!(estimate_latency(44_100, 512).zone, LatencyZone::Red);
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());

        let s = AudioSettings {
            input_device: Some("Focusrite Scarlett 4i4".to_string()),
            sample_rate: 96_000,
            buffer_size: 128,
            ..Default::default()
        };
        save(&path, &s).expect("save ok");

        assert_eq!(load(&path), s);
    }

    #[test]
    fn missing_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        assert_eq!(load(&path), AudioSettings::default());
    }

    #[test]
    fn corrupt_file_yields_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        std::fs::write(&path, b"{ not valid json").unwrap();
        assert_eq!(load(&path), AudioSettings::default());
    }

    #[test]
    fn save_refuses_invalid_settings() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        let s = AudioSettings {
            sample_rate: 1,
            ..Default::default()
        };
        assert!(save(&path, &s).is_err());
        assert!(
            !path.exists(),
            "nothing should be written on validation error"
        );
    }
}
