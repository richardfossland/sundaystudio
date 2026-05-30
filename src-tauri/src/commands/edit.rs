//! Editing / timeline commands (Phase 3.1).
//!
//! The editor reads the timeline (`project_timeline`), draws each clip from
//! precomputed waveform peaks (`audio_peaks`), and persists every move/trim/fade
//! as a region mutation (`region_*`). All region edits are non-destructive — they
//! only touch the `region` rows; the take WAVs are never rewritten.
//!
//! Until live multi-track capture is wired on real hardware, `take_import` is how
//! audio gets onto the timeline: it lays existing WAVs onto tracks 1:1 as a new
//! take, which is also exactly the shape the recorder will produce. That makes
//! the whole editor exercisable today, and gives `export_render` a take to bounce.

use std::path::{Path, PathBuf};

use tauri::State;

use crate::audio::peaks::{self, WaveformPeaks};
use crate::commands::project::{current, ProjectState};
use crate::dsp::silence::{self, SilenceSpan};
use crate::error::{AppError, AppResult};
use crate::export::render::read_wav_mono;
use crate::project::model::{Region, TimelineSnapshot};
use crate::project::{scast, store};

/// Colours handed to tracks auto-created by an import (cycled by index).
const IMPORT_COLORS: [&str; 6] = [
    "#D4A73A", "#3A8DD4", "#4CB97A", "#D47A3A", "#9B6BD4", "#D44A6B",
];

/// The open project's takes and placed regions (tracks + markers come from
/// `project_snapshot`).
#[tauri::command]
pub async fn project_timeline(state: State<'_, ProjectState>) -> AppResult<TimelineSnapshot> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::load_timeline(&op.pool, &op.project_id).await
}

/// A take track's waveform overview for the timeline, computed + cached on first
/// request. File IO runs on a blocking thread so the async runtime stays free.
#[tauri::command]
pub async fn audio_peaks(
    state: State<'_, ProjectState>,
    take_id: String,
    source_track_id: String,
) -> AppResult<WaveformPeaks> {
    let (scast_dir, take_dir) = {
        let guard = state.current.lock().await;
        let op = current(&guard)?;
        (
            op.scast_dir.clone(),
            scast::take_dir(&op.scast_dir, &take_id),
        )
    };
    tokio::task::spawn_blocking(move || {
        peaks::load_or_compute(&scast_dir, &take_dir, &take_id, &source_track_id)
    })
    .await
    .map_err(|e| AppError::Internal(format!("peaks task failed: {e}")))?
}

/// Place a new region on a track, with the default 5 ms anti-click fades.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn region_add(
    state: State<'_, ProjectState>,
    take_id: String,
    source_track_id: String,
    target_track_id: String,
    start_in_take_ms: f64,
    end_in_take_ms: f64,
    position_in_timeline_ms: f64,
) -> AppResult<Region> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::add_region(
        &op.pool,
        Region {
            id: String::new(),
            take_id,
            source_track_id,
            target_track_id,
            start_in_take_ms,
            end_in_take_ms,
            position_in_timeline_ms,
            fade_in_ms: 5.0,
            fade_out_ms: 5.0,
            gain_adjust_db: 0.0,
        },
    )
    .await
}

/// Detect silent gaps in a take track's audio (take-relative ms), for the editor
/// to trim. `threshold_db` is dBFS (default ≈ −50), `min_silence_ms` the shortest
/// gap to report (default ≈ 500). File IO + scan run on a blocking thread.
#[tauri::command]
pub async fn analyze_silence(
    state: State<'_, ProjectState>,
    take_id: String,
    source_track_id: String,
    threshold_db: f64,
    min_silence_ms: f64,
) -> AppResult<Vec<SilenceSpan>> {
    let wav = {
        let guard = state.current.lock().await;
        let op = current(&guard)?;
        scast::take_dir(&op.scast_dir, &take_id).join(format!("{source_track_id}.wav"))
    };
    if !wav.exists() {
        return Err(AppError::NotFound {
            entity: "take audio",
            id: source_track_id,
        });
    }
    tokio::task::spawn_blocking(move || {
        let (samples, rate) = read_wav_mono(&wav).map_err(AppError::Audio)?;
        Ok(silence::detect_silences(
            &samples,
            rate,
            threshold_db as f32,
            min_silence_ms,
        ))
    })
    .await
    .map_err(|e| AppError::Internal(format!("silence scan failed: {e}")))?
}

/// Insert a region with a caller-supplied id. Used by edits like split and
/// undo/redo, where the frontend mints the id up front so the operation has a
/// stable, reversible inverse (`region_delete(id)` / re-`region_create`).
#[tauri::command]
pub async fn region_create(state: State<'_, ProjectState>, region: Region) -> AppResult<Region> {
    if region.id.is_empty() {
        return Err(AppError::Validation("region id required".into()));
    }
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::add_region(&op.pool, region).await
}

/// Persist a region edit (move, trim, fade, gain, or retarget to another track).
#[tauri::command]
pub async fn region_update(state: State<'_, ProjectState>, region: Region) -> AppResult<()> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::update_region(&op.pool, &region).await
}

/// Delete a region (the clip leaves the timeline; its take is untouched).
#[tauri::command]
pub async fn region_delete(state: State<'_, ProjectState>, id: String) -> AppResult<()> {
    let guard = state.current.lock().await;
    let op = current(&guard)?;
    store::delete_region(&op.pool, &id).await
}

/// Import existing WAVs onto the timeline as a new take. Each file lands 1:1 on a
/// track (reusing the project's tracks in order, creating new ones for overflow),
/// with a region covering its full length at the timeline origin. Returns the
/// refreshed timeline; the caller should also refetch the project snapshot since
/// tracks may have been added.
#[tauri::command]
pub async fn take_import(
    state: State<'_, ProjectState>,
    paths: Vec<String>,
) -> AppResult<TimelineSnapshot> {
    if paths.is_empty() {
        return Err(AppError::Validation("no files to import".into()));
    }

    let guard = state.current.lock().await;
    let op = current(&guard)?;
    let project = store::load_project(&op.pool).await?;
    let project_rate = project.sample_rate as u32;

    // Probe every file up front: reject anything unreadable or off-rate before we
    // create a take, so a bad import leaves the project untouched.
    let mut probed: Vec<(PathBuf, f64)> = Vec::with_capacity(paths.len());
    for p in &paths {
        let path = PathBuf::from(p);
        let (duration_ms, rate) = probe_wav(&path)?;
        if rate != project_rate {
            return Err(AppError::Validation(format!(
                "{} is {rate} Hz but the project is {project_rate} Hz — sample-rate conversion lands in a later phase",
                path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
            )));
        }
        probed.push((path, duration_ms));
    }

    // Resolve a target track per file: reuse existing tracks in order, then append.
    let mut tracks = store::list_tracks(&op.pool, &op.project_id).await?;
    let mut target_ids: Vec<String> = Vec::with_capacity(probed.len());
    for (i, (path, _)) in probed.iter().enumerate() {
        if let Some(t) = tracks.get(i) {
            target_ids.push(t.id.clone());
        } else {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| format!("Track {}", i + 1));
            let color = IMPORT_COLORS[tracks.len() % IMPORT_COLORS.len()];
            let added = store::add_track(&op.pool, &op.project_id, &name, color).await?;
            target_ids.push(added.id.clone());
            tracks.push(added);
        }
    }

    let total_ms = probed.iter().map(|(_, d)| *d).fold(0.0_f64, f64::max);
    let take = store::add_take(
        &op.pool,
        &op.project_id,
        store::now_ms(),
        total_ms,
        &target_ids,
    )
    .await?;

    // Copy each WAV into the take folder under its target track id, then place a
    // region covering its whole length at the timeline origin.
    let dir = scast::take_dir(&op.scast_dir, &take.id);
    std::fs::create_dir_all(&dir)?;
    for ((path, duration_ms), track_id) in probed.iter().zip(&target_ids) {
        std::fs::copy(path, dir.join(format!("{track_id}.wav")))?;
        store::add_region(
            &op.pool,
            Region {
                id: String::new(),
                take_id: take.id.clone(),
                source_track_id: track_id.clone(),
                target_track_id: track_id.clone(),
                start_in_take_ms: 0.0,
                end_in_take_ms: *duration_ms,
                position_in_timeline_ms: 0.0,
                fade_in_ms: 5.0,
                fade_out_ms: 5.0,
                gain_adjust_db: 0.0,
            },
        )
        .await?;
    }

    store::load_timeline(&op.pool, &op.project_id).await
}

/// Read a WAV's duration (ms) and sample rate from its header — no full decode.
fn probe_wav(path: &Path) -> AppResult<(f64, u32)> {
    let reader = hound::WavReader::open(path)
        .map_err(|e| AppError::Audio(format!("open {}: {e}", path.display())))?;
    let spec = reader.spec();
    let frames = reader.duration(); // samples per channel
    let rate = spec.sample_rate;
    let ms = frames as f64 / rate.max(1) as f64 * 1000.0;
    Ok((ms, rate))
}
