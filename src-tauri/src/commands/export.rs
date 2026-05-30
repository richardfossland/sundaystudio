//! Export commands (Phase 7.1).
//!
//! `export_presets` lists the platform-ready presets; `export_render` bounces the
//! open project's latest take to a mastered, loudness-normalised 24-bit WAV in
//! the project's `exports/` folder. The heavy mix + DSP + file IO runs on a
//! blocking thread so the async runtime stays responsive.
//!
//! MP3/AAC/FLAC encoding (the ffmpeg sidecar) is Phase 7.1b: for those presets
//! we still write the master WAV and say so in `note`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::State;
use ts_rs::TS;

use crate::commands::project::{current, ProjectState};
use crate::dsp::chain::Preset as VoicePreset;
use crate::dsp::loudness::{self, LoudnessTarget, NormalizationReport};
use crate::dsp::master::MasterPreset;
use crate::dsp::Effect;
use crate::error::{AppError, AppResult};
use crate::export::format::{self, ExportFormat, ExportPresetInfo};
use crate::export::render::{self, MixSource, PlacedClip};
use crate::project::{scast, store};

/// One clip to bake into a track's timeline during export.
struct ClipPlan {
    wav_path: PathBuf,
    start_ms: f64,
    end_ms: f64,
    fade_in_ms: f64,
    fade_out_ms: f64,
    gain_db: f32,
    position_ms: f64,
}

/// One track's export plan: its clips plus the track's mixer state.
struct TrackPlan {
    gain_db: f32,
    mute: bool,
    /// Voice-processing preset id to apply to the track before mixing (if any).
    voice_preset: Option<String>,
    clips: Vec<ClipPlan>,
}

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

    // Resolve the timeline (tracks + their placed regions) from the open project.
    // Export is region-aware: each clip is trimmed, gained and faded at its
    // timeline position, so what you arranged in the editor is what bounces.
    let (plans, rate, out_path) = {
        let guard = state.current.lock().await;
        let op = current(&guard)?;
        let project = store::load_project(&op.pool).await?;
        let tracks = store::list_tracks(&op.pool, &op.project_id).await?;
        let regions = store::list_project_regions(&op.pool, &op.project_id).await?;
        if regions.is_empty() {
            return Err(AppError::Validation(
                "nothing to export yet — import or record audio, then arrange clips on the timeline".into(),
            ));
        }

        let soloed = tracks.iter().any(|t| t.solo);
        let mut plans: Vec<TrackPlan> = Vec::new();
        for t in &tracks {
            let clips: Vec<ClipPlan> = regions
                .iter()
                .filter(|r| r.target_track_id == t.id)
                .map(|r| ClipPlan {
                    wav_path: scast::take_dir(&op.scast_dir, &r.take_id)
                        .join(format!("{}.wav", r.source_track_id)),
                    start_ms: r.start_in_take_ms,
                    end_ms: r.end_in_take_ms,
                    fade_in_ms: r.fade_in_ms,
                    fade_out_ms: r.fade_out_ms,
                    gain_db: r.gain_adjust_db as f32,
                    position_ms: r.position_in_timeline_ms,
                })
                .collect();
            if clips.is_empty() {
                continue; // this track has no clips on the timeline
            }
            let active = if soloed { t.solo } else { true };
            plans.push(TrackPlan {
                gain_db: t.gain_db as f32,
                mute: t.mute || !active,
                voice_preset: t.voice_preset.clone(),
                clips,
            });
        }
        if plans.is_empty() || plans.iter().all(|p| p.mute) {
            return Err(AppError::Validation(
                "nothing audible to export — every track is muted or empty".into(),
            ));
        }

        let exports_dir = op.scast_dir.join("exports");
        fs::create_dir_all(&exports_dir)?;
        let stem = sanitize_stem(&project.name);
        let out_path = exports_dir.join(format!("{stem}.wav"));
        (plans, project.sample_rate as u32, out_path)
    };

    let channels = preset.channels;

    // Mixing, DSP and file IO are CPU/blocking — keep them off the async runtime.
    tokio::task::spawn_blocking(move || {
        render_and_write(plans, &out_path, channels, rate, master, &target, &preset, &preset_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("export task failed: {e}")))?
}

/// Assemble each track's timeline from its clips, mix, master, write the WAV, and
/// build the result. Sync — runs inside `spawn_blocking`. Each source WAV is
/// decoded once and reused across the clips that reference it (e.g. both halves
/// of a split).
#[allow(clippy::too_many_arguments)]
fn render_and_write(
    plans: Vec<TrackPlan>,
    out_path: &Path,
    channels: u16,
    rate: u32,
    master: MasterPreset,
    target: &LoudnessTarget,
    preset: &ExportPresetInfo,
    preset_id: &str,
) -> AppResult<ExportResult> {
    let mut cache: HashMap<PathBuf, Vec<f32>> = HashMap::new();
    let mut sources = Vec::with_capacity(plans.len());
    for plan in &plans {
        let mut clips: Vec<PlacedClip> = Vec::with_capacity(plan.clips.len());
        for clip in &plan.clips {
            if !clip.wav_path.exists() {
                continue; // a region whose audio is missing — skip, don't abort
            }
            if !cache.contains_key(&clip.wav_path) {
                let (samples, src_rate) =
                    render::read_wav_mono(&clip.wav_path).map_err(AppError::Audio)?;
                if src_rate != rate {
                    return Err(AppError::Validation(format!(
                        "clip audio is {src_rate} Hz but the project is {rate} Hz — sample-rate conversion lands in a later phase"
                    )));
                }
                cache.insert(clip.wav_path.clone(), samples);
            }
            let source = &cache[&clip.wav_path];
            let samples = render::render_region(
                source,
                rate,
                clip.start_ms,
                clip.end_ms,
                clip.fade_in_ms,
                clip.fade_out_ms,
                clip.gain_db,
            );
            clips.push(PlacedClip { position_ms: clip.position_ms, samples });
        }
        let mut track_buf = render::assemble_timeline(&clips, rate);
        // Apply the track's bundled voice chain (gate → EQ → de-ess → comp → sat)
        // to the assembled timeline before it hits the mix bus.
        if let Some(preset) = plan.voice_preset.as_deref().and_then(VoicePreset::from_id) {
            let mut chain = preset.build();
            chain.prepare(rate as f32);
            chain.process(&mut track_buf);
        }
        sources.push(MixSource {
            samples: track_buf,
            gain_db: plan.gain_db,
            mute: plan.mute,
        });
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
