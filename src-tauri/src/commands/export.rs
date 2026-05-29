//! Export commands (Phase 7.1).
//!
//! `export_presets` lists the platform-ready presets; `export_render` bounces the
//! open project's latest take to a mastered, loudness-normalised 24-bit WAV in
//! the project's `exports/` folder. The heavy mix + DSP + file IO runs on a
//! blocking thread so the async runtime stays responsive.
//!
//! MP3/AAC/FLAC encoding (the ffmpeg sidecar) is Phase 7.1b: for those presets
//! we still write the master WAV and say so in `note`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::commands::project::{current, ProjectState};
use crate::dsp::loudness::{self, LoudnessTarget, NormalizationReport};
use crate::dsp::master::MasterPreset;
use crate::error::{AppError, AppResult};
use crate::export::format::{self, ExportFormat, ExportPresetInfo};
use crate::export::render::{self, MixSource};
use crate::project::{scast, store};

/// The outcome of an export render, returned to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ExportResult.ts")]
pub struct ExportResult {
    /// Absolute path of the file written.
    pub output_path: String,
    /// The format actually written (WAV today, even for encoder presets).
    pub written_format: ExportFormat,
    /// The export preset the user picked.
    pub requested_preset_id: String,
    /// File size in bytes.
    pub bytes: f64,
    /// Programme duration in milliseconds.
    pub duration_ms: f64,
    pub channels: u16,
    pub sample_rate: u32,
    /// Before/after loudness measurement of the bounce.
    pub loudness: NormalizationReport,
    /// True when the achieved integrated loudness is within 0.5 LU of target
    /// (the plan's verification threshold).
    pub target_reached: bool,
    /// Human-readable caveat (encoder pending, or loudness off target).
    pub note: Option<String>,
}

/// The platform-ready export presets (format + bitrate + channels + LUFS target).
#[tauri::command]
pub fn export_presets() -> AppResult<Vec<ExportPresetInfo>> {
    Ok(format::export_presets())
}

/// Bounce the open project's latest take to a mastered, normalised WAV. Pass an
/// optional mastering-preset id (defaults to "conversation-podcast").
#[tauri::command]
pub async fn export_render(
    state: State<'_, ProjectState>,
    preset_id: String,
    master_preset_id: Option<String>,
) -> AppResult<ExportResult> {
    let preset = format::preset_by_id(&preset_id)
        .ok_or_else(|| AppError::Validation(format!("unknown export preset: {preset_id}")))?;
    let target = loudness::target_by_id(&preset.target_id).ok_or_else(|| {
        AppError::Internal(format!("preset references unknown target {}", preset.target_id))
    })?;
    let master = master_preset_id
        .as_deref()
        .and_then(MasterPreset::from_id)
        .unwrap_or(MasterPreset::ConversationPodcast);

    // Resolve the latest take's per-track WAVs from the open project (async DB).
    let (inputs, rate, out_path) = {
        let guard = state.current.lock().await;
        let op = current(&guard)?;
        let project = store::load_project(&op.pool).await?;
        let tracks = store::list_tracks(&op.pool, &op.project_id).await?;
        let mut takes = store::list_takes(&op.pool, &op.project_id).await?;
        // Most recent take first.
        takes.sort_by(|a, b| {
            b.started_at
                .partial_cmp(&a.started_at)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let take = takes
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Validation("nothing to export yet — record a take first".into()))?;
        let dir = scast::take_dir(&op.scast_dir, &take.id);

        let soloed = tracks.iter().any(|t| t.solo);
        let mut inputs: Vec<(PathBuf, f32, bool)> = Vec::new();
        for t in &tracks {
            let path = dir.join(format!("{}.wav", t.id));
            if !path.exists() {
                continue; // this track captured no audio in the take
            }
            let active = if soloed { t.solo } else { true };
            inputs.push((path, t.gain_db as f32, t.mute || !active));
        }
        if inputs.iter().all(|(_, _, muted)| *muted) {
            return Err(AppError::Validation(
                "nothing audible to export in the latest take".into(),
            ));
        }

        let exports_dir = op.scast_dir.join("exports");
        fs::create_dir_all(&exports_dir)?;
        let stem = sanitize_stem(&project.name);
        let out_path = exports_dir.join(format!("{stem}.wav"));
        (inputs, project.sample_rate as u32, out_path)
    };

    let channels = preset.channels;

    // Mixing, DSP and file IO are CPU/blocking — keep them off the async runtime.
    tokio::task::spawn_blocking(move || {
        render_and_write(inputs, &out_path, channels, rate, master, &target, &preset, &preset_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("export task failed: {e}")))?
}

/// Read the sources, render, write the WAV, and assemble the result. Sync —
/// runs inside `spawn_blocking`.
#[allow(clippy::too_many_arguments)]
fn render_and_write(
    inputs: Vec<(PathBuf, f32, bool)>,
    out_path: &Path,
    channels: u16,
    rate: u32,
    master: MasterPreset,
    target: &LoudnessTarget,
    preset: &ExportPresetInfo,
    preset_id: &str,
) -> AppResult<ExportResult> {
    let mut sources = Vec::with_capacity(inputs.len());
    for (path, gain_db, mute) in inputs {
        let (samples, src_rate) = render::read_wav_mono(&path).map_err(AppError::Audio)?;
        if src_rate != rate {
            return Err(AppError::Validation(format!(
                "take audio is {src_rate} Hz but the project is {rate} Hz — sample-rate conversion lands in a later phase"
            )));
        }
        sources.push(MixSource { samples, gain_db, mute });
    }

    let (out, report) = render::render(&sources, channels, rate, master, target)?;
    let bytes = render::write_wav(out_path, &out, channels, rate, 24).map_err(AppError::Audio)?;

    let frames = out.len() / channels.max(1) as usize;
    let duration_ms = frames as f64 / rate as f64 * 1000.0;
    let target_reached = report
        .after
        .integrated_lufs
        .map(|l| (l - target.integrated_lufs).abs() <= 0.5)
        .unwrap_or(false);

    let note = if preset.requires_encoder {
        Some(format!(
            "Wrote a 24-bit master WAV. {} encoding lands once the ffmpeg sidecar is bundled (Phase 7.1b).",
            preset.format.extension().to_uppercase()
        ))
    } else if !target_reached {
        Some("Loudness landed more than 0.5 LU off target — check the mix.".to_string())
    } else {
        None
    };

    Ok(ExportResult {
        output_path: out_path.display().to_string(),
        written_format: ExportFormat::Wav,
        requested_preset_id: preset_id.to_string(),
        bytes: bytes as f64,
        duration_ms,
        channels,
        sample_rate: rate,
        loudness: report,
        target_reached,
        note,
    })
}

/// Make a filesystem-safe file stem from a project name.
fn sanitize_stem(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' })
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "export".to_string()
    } else {
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_stem_keeps_safe_chars_and_falls_back() {
        assert_eq!(sanitize_stem("Sunday Morning"), "Sunday Morning");
        assert_eq!(sanitize_stem("Ep 12: Q&A / part 1"), "Ep 12_ Q_A _ part 1");
        assert_eq!(sanitize_stem("   "), "export"); // all whitespace → fallback
        assert_eq!(sanitize_stem("///"), "___"); // non-empty after substitution
    }
}
