//! Test-tone synthesis + WAV writing.
//!
//! The Phase 0.1 smoke test for the *output* half of the audio story: synthesise
//! a 1-second sine wave and write it to disk as a canonical 16-bit PCM WAV via
//! `hound`. If this round-trips on both platforms, our WAV writing path — which
//! Phase 1.2 turns into the continuous multi-track recorder — is sound.
//!
//! Deliberately not played back yet: Phase 0.1 only needs bytes on disk. Live
//! monitoring/playback is the recorder's job (Phase 1.2/1.3).

use std::f32::consts::TAU;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};

/// What `write_test_tone` produced — surfaced to the UI as proof of life.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../src/lib/bindings/ToneResult.ts")]
pub struct ToneResult {
    /// Absolute path to the written WAV file.
    pub path: String,
    /// Sample rate the tone was written at (Hz).
    pub sample_rate: u32,
    /// Tone frequency (Hz).
    pub frequency: f32,
    /// Duration in milliseconds.
    pub duration_ms: u32,
    /// Final file size in bytes.
    pub bytes: u64,
}

/// Synthesise a sine wave and write it to `path` as 16-bit mono PCM WAV.
///
/// Pulled out of the command so it is unit-testable without Tauri: returns the
/// `ToneResult` describing exactly what hit disk.
pub fn write_test_tone(
    path: &Path,
    sample_rate: u32,
    frequency: f32,
    duration_ms: u32,
) -> AppResult<ToneResult> {
    if sample_rate == 0 {
        return Err(AppError::Validation("sample_rate must be > 0".into()));
    }
    if !(frequency.is_finite() && frequency > 0.0) {
        return Err(AppError::Validation("frequency must be positive".into()));
    }

    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };

    let mut writer = hound::WavWriter::create(path, spec)
        .map_err(|e| AppError::Audio(format!("creating WAV {}: {e}", path.display())))?;

    let total_samples = (sample_rate as u64 * duration_ms as u64 / 1000) as u32;
    // -6 dBFS so the tone is unmistakably audible without clipping.
    let amplitude = i16::MAX as f32 * 0.5;

    for n in 0..total_samples {
        let t = n as f32 / sample_rate as f32;
        let sample = (frequency * TAU * t).sin() * amplitude;
        writer
            .write_sample(sample as i16)
            .map_err(|e| AppError::Audio(format!("writing sample: {e}")))?;
    }

    writer
        .finalize()
        .map_err(|e| AppError::Audio(format!("finalising WAV: {e}")))?;

    let bytes = std::fs::metadata(path)?.len();

    Ok(ToneResult {
        path: path.to_string_lossy().into_owned(),
        sample_rate,
        frequency,
        duration_ms,
        bytes,
    })
}

/// Default location for the smoke-test tone: the OS temp dir. Keeps Phase 0.1
/// free of any project/storage concepts (those arrive in Phase 2.1).
pub fn default_tone_path() -> PathBuf {
    std::env::temp_dir().join("sundaystudio-test-tone.wav")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_one_second_440hz_tone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");

        let res = write_test_tone(&path, 48_000, 440.0, 1000).expect("tone writes");

        assert!(path.exists(), "WAV file should exist on disk");
        assert_eq!(res.sample_rate, 48_000);
        assert_eq!(res.duration_ms, 1000);
        // 48000 samples * 2 bytes + 44-byte canonical header.
        assert_eq!(res.bytes, 48_000 * 2 + 44);

        // Reading it back proves the file is a valid, complete WAV.
        let reader = hound::WavReader::open(&path).expect("reopen WAV");
        assert_eq!(reader.spec().sample_rate, 48_000);
        assert_eq!(reader.spec().channels, 1);
        assert_eq!(reader.len(), 48_000);
    }

    #[test]
    fn rejects_invalid_parameters() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.wav");
        assert!(write_test_tone(&path, 0, 440.0, 1000).is_err());
        assert!(write_test_tone(&path, 48_000, -1.0, 1000).is_err());
    }
}
