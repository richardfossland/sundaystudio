//! DSP commands.
//!
//! Phase 4.1 exposed the bundled voice presets so the UI can list them. Phase
//! 4.2 adds loudness: the platform normalisation targets and an analyser that
//! measures any WAV's LUFS / true-peak (so a finished take can show "you're at
//! -19 LUFS, Spotify wants -14"). Applying a master chain / live processing
//! arrives with the mixer.

use std::path::PathBuf;

use crate::dsp::chain::{preset_infos, PresetInfo};
use crate::dsp::loudness::{self, LoudnessMeasurement, LoudnessTarget};
use crate::dsp::master::{master_preset_infos, MasterPresetInfo};
use crate::error::{AppError, AppResult};

/// The bundled voice-processing presets (Voice / Bright Voice / Warm Voice /
/// Broadcast). All free; the AI "Smart Preset" that picks one is Pro (Phase 4.3).
#[tauri::command]
pub fn dsp_presets() -> AppResult<Vec<PresetInfo>> {
    Ok(preset_infos())
}

/// The "Match to platform" loudness targets (Spotify / Apple Podcasts / YouTube
/// / Broadcast). Each carries its integrated-LUFS goal and true-peak ceiling.
#[tauri::command]
pub fn dsp_loudness_targets() -> AppResult<Vec<LoudnessTarget>> {
    Ok(loudness::loudness_targets())
}

/// The bundled mastering presets (Conversation Podcast / Sermon / Music-heavy /
/// Loud & Bright). Each pairs a master chain with the platform target it
/// normalises to.
#[tauri::command]
pub fn dsp_master_presets() -> AppResult<Vec<MasterPresetInfo>> {
    Ok(master_preset_infos())
}

/// Measure the loudness (integrated/short/momentary LUFS, range, true & sample
/// peak) of a WAV file on disk. Used by the analysis UI and, later, export.
#[tauri::command]
pub fn dsp_analyze_file(path: String) -> AppResult<LoudnessMeasurement> {
    let (samples, channels, rate) = read_wav_interleaved(PathBuf::from(path))?;
    Ok(loudness::measure(&samples, channels, rate)?)
}

/// Read a WAV into interleaved f32 in [-1, 1], with its channel count and rate.
/// Integer PCM is scaled by its bit depth; float WAVs pass through.
fn read_wav_interleaved(path: PathBuf) -> AppResult<(Vec<f32>, u32, u32)> {
    let mut reader = hound::WavReader::open(&path)
        .map_err(|e| AppError::Audio(format!("open {}: {e}", path.display())))?;
    let spec = reader.spec();

    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| AppError::Audio(format!("decode float WAV: {e}")))?,
        hound::SampleFormat::Int => {
            // i32 covers every integer bit depth hound yields; scale to ±1.0.
            let scale = 1.0_f32 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| AppError::Audio(format!("decode int WAV: {e}")))?
        }
    };

    Ok((samples, spec.channels as u32, spec.sample_rate))
}
