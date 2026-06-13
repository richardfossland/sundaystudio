//! Audio commands — the Phase 0.1 smoke test of the highest-risk subsystem.
//!
//! `audio_devices` enumerates the system's input/output devices (proves cpal
//! links and talks to CoreAudio/WASAPI). `audio_record_test_tone` writes a
//! 1-second sine WAV to disk (proves our WAV writing path works). Together they
//! exercise the device layer and the file layer before Phase 1 builds the
//! real-time recording engine on top.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;
use ts_rs::TS;

use crate::ai::jingle::{self, JingleResult, JingleSpec};
use crate::ai::leveling::{self, LevelingResult, LevelingSnapshot, LevelingTrack};
use crate::ai::shownotes::{self, ShowNotes, ShowNotesInput};
use crate::ai::{self, ReqwestTransport};
use crate::audio::playback::{self, PlaybackController, PlaybackTrack};
use crate::audio::recorder::{
    start_session, RecordConfig, RecordController, RecorderCommand, StreamHandle, TrackSpec,
};
use crate::audio::{devices, settings, tone};
use crate::commands::project::{current, ProjectState};
use crate::dsp::loudness;
use crate::error::{AppError, AppResult};
use crate::export::render::{self, PlacedClip};
use crate::project::{scast, store};

/// A live recording session: the tested `RecordController` (rings + writer
/// thread + meters) plus the `StreamHandle` that owns the `!Send` cpal input
/// stream on its own thread, and the metadata we need to lay the captured WAVs
/// onto the project timeline when the take ends.
///
/// Kept together so `RecorderControl` can stop both halves in the right order
/// (stream first → no more samples arrive → then drain + finalise the writer).
struct RecorderSession {
    controller: RecordController,
    stream: StreamHandle,
    take_id: String,
    project_id: String,
    scast_dir: PathBuf,
    sample_rate: u32,
    /// Project track ids this take captured into, channel order = track order.
    /// These are also the per-track WAV filenames in the take dir.
    track_ids: Vec<String>,
    /// Wall-clock start (ms) for the take row.
    started_at: f64,
}

/// Tauri-managed handle to the live recording transport.
///
/// Mirrors [`PlaybackControl`]: `audio_record_start` resolves the input device,
/// starts the tested session pipeline, opens the cpal input stream against it,
/// and installs the [`RecorderSession`] here; `audio_record_stop` tears the
/// stream + writer down and lays the captured audio onto the timeline;
/// `audio_record_status` is what the UI polls (~60fps) for the recording state,
/// the live take duration, per-channel meters and any overruns.
///
/// HARDWARE-UNVERIFIED: the session pipeline (rings → writer → WAVs → meters) is
/// fully tested without a device (see `tests/recording_transport.rs`), but the
/// cpal **input** stream that feeds it — opened by [`StreamHandle::spawn`] — is
/// the one hardware-dependent piece, validated on real interfaces in Phase 2.2.
#[derive(Default)]
pub struct RecorderControl {
    session: Mutex<Option<RecorderSession>>,
}

impl RecorderControl {
    /// Is a take currently rolling?
    async fn is_recording(&self) -> bool {
        self.session.lock().await.is_some()
    }

    /// Enqueue a control command for the live take's audio thread (monitoring /
    /// monitor-mute). Errors cleanly if no take is rolling, or if the lock-free
    /// command queue is momentarily full (the UI can retry).
    async fn send_command(&self, cmd: RecorderCommand) -> AppResult<()> {
        let mut guard = self.session.lock().await;
        let session = guard
            .as_mut()
            .ok_or_else(|| AppError::Validation("no active recording session to monitor".into()))?;
        if session.controller.send_command(cmd) {
            Ok(())
        } else {
            Err(AppError::Audio("monitor command queue full; retry".into()))
        }
    }
}

/// What the UI polls while the transport is wired: whether a take is rolling,
/// the live captured-frame count (→ duration) and overruns, plus the per-channel
/// peak meters in dBFS (one entry per captured channel).
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/RecordingStatus.ts")]
pub struct RecordingStatus {
    /// Whether a take is currently being captured.
    pub recording: bool,
    /// Frames captured so far (one per per-channel sample); 0 when idle.
    pub captured_frames: f64,
    /// Captured duration in milliseconds, derived from `captured_frames`.
    pub duration_ms: f64,
    /// Samples dropped to ring overruns so far (0 is healthy).
    pub dropped: f64,
    /// Per-channel peak in dBFS since the last poll (UI meters). Empty when idle.
    pub meters_dbfs: Vec<f32>,
    /// `true` if the writer thread died mid-take (a disk error): capture may
    /// still be running and meters live, but nothing is reaching disk — the take
    /// is being lost. The UI must surface this immediately ("recording is
    /// sacred"). Always `false` when idle or healthy.
    pub writer_failed: bool,
}

impl RecordingStatus {
    /// The idle status (nothing recording) — what `audio_record_status` returns
    /// when no take is rolling.
    fn idle() -> Self {
        RecordingStatus {
            recording: false,
            captured_frames: 0.0,
            duration_ms: 0.0,
            dropped: 0.0,
            meters_dbfs: Vec::new(),
            writer_failed: false,
        }
    }
}

/// Enumerate input and output audio devices on the default host.
#[tauri::command]
pub fn audio_devices() -> AppResult<devices::AudioDeviceList> {
    devices::enumerate()
}

/// Synthesise a 1-second 440 Hz sine wave and write it to the OS temp dir as a
/// 16-bit mono WAV. Returns where it landed and how big it is.
#[tauri::command]
pub fn audio_record_test_tone() -> AppResult<tone::ToneResult> {
    let path = tone::default_tone_path();
    tone::write_test_tone(&path, 48_000, 440.0, 1000)
}

/// Resolve the app config dir, mapping a missing-dir error into our domain type.
fn config_dir(app: &AppHandle) -> AppResult<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| AppError::Internal(format!("resolving config dir: {e}")))
}

/// Load the persisted audio settings (defaults on first run / corrupt file).
#[tauri::command]
pub fn audio_get_settings(app: AppHandle) -> AppResult<settings::AudioSettings> {
    let path = settings::settings_path(&config_dir(&app)?);
    Ok(settings::load(&path))
}

/// Persist audio settings after validating them.
#[tauri::command]
pub fn audio_set_settings(app: AppHandle, new_settings: settings::AudioSettings) -> AppResult<()> {
    let path = settings::settings_path(&config_dir(&app)?);
    settings::save(&path, &new_settings)
}

/// Estimate round-trip monitoring latency for a sample-rate / buffer choice.
/// Pure — no device or app state needed, so the settings UI can call it live as
/// the user drags the buffer selector.
#[tauri::command]
pub fn audio_latency_estimate(
    sample_rate: u32,
    buffer_size: u32,
) -> AppResult<settings::LatencyEstimate> {
    Ok(settings::estimate_latency(sample_rate, buffer_size))
}

/// Turn low-latency software monitoring on or off (Phase 1.3). Enqueues a
/// `SetMonitoring` for the live take's audio thread, which starts/stops feeding
/// the mono monitor mix to the output callback. Errors if no take is recording.
#[tauri::command]
pub async fn audio_set_monitoring(
    recorder: State<'_, RecorderControl>,
    enabled: bool,
) -> AppResult<()> {
    recorder
        .send_command(RecorderCommand::SetMonitoring(enabled))
        .await
}

/// Mute/unmute one track in the monitor mix without affecting capture (Phase
/// 1.3). `track_idx` is the input-channel index. Errors if no take is live.
#[tauri::command]
pub async fn audio_set_monitor_mute(
    recorder: State<'_, RecorderControl>,
    track_idx: usize,
    muted: bool,
) -> AppResult<()> {
    recorder
        .send_command(RecorderCommand::SetMute {
            track: track_idx,
            muted,
        })
        .await
}

/// Start recording the open project's tracks (Phase 2.2 transport).
///
/// Resolves the input device (None = host default), arms one capture track per
/// existing project track (the channel→track map is 1:1 for now), starts the
/// tested session pipeline, then opens the cpal input stream against it. The
/// take owns the audio thread's command queue, so the monitoring commands
/// (`audio_set_monitoring`, `audio_set_monitor_mute`) reach it through
/// [`RecorderControl`]. Errors — including a missing device or an unsupported
/// stream format — surface here, leaving nothing installed.
///
/// `device_name` is the OS device name from `audio_devices`; `channels` lets the
/// caller capture more interleaved input channels than there are tracks (extra
/// channels are dropped). When omitted it defaults to the project's track count.
#[tauri::command]
pub async fn audio_record_start(
    project: State<'_, ProjectState>,
    recorder: State<'_, RecorderControl>,
    device_name: Option<String>,
    channels: Option<u16>,
) -> AppResult<RecordingStatus> {
    if recorder.is_recording().await {
        return Err(AppError::Validation(
            "a recording is already in progress — stop it first".into(),
        ));
    }

    // Snapshot what we need from the open project, then drop its lock before any
    // blocking device work.
    let (project_id, scast_dir, sample_rate, track_ids) = {
        let guard = project.current.lock().await;
        let op = current(&guard)?;
        let proj = store::load_project(&op.pool).await?;
        let tracks = store::list_tracks(&op.pool, &op.project_id).await?;
        let ids: Vec<String> = tracks.into_iter().map(|t| t.id).collect();
        (
            op.project_id.clone(),
            op.scast_dir.clone(),
            proj.sample_rate as u32,
            ids,
        )
    };

    if track_ids.is_empty() {
        return Err(AppError::Validation(
            "add at least one track before recording".into(),
        ));
    }

    // Capture as many channels as the caller asked for (default: one per track).
    // Tracks beyond the channel count simply receive no audio this take.
    let channel_count = channels.map(|c| c as usize).unwrap_or(track_ids.len());
    let started_at = store::now_ms();
    let take_id = store::new_take_id();
    let take_dir = scast::take_dir(&scast_dir, &take_id);

    // Pre-create the session: each capture track writes `<track_id>.wav` into the
    // take dir, so the on-disk filenames line up with the project track ids the
    // regions will reference (exactly as `import_takes` arranges them).
    let session_track_ids: Vec<String> = (0..channel_count)
        .map(|i| {
            track_ids
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("ch{i}"))
        })
        .collect();
    let tracks: Vec<TrackSpec> = session_track_ids
        .iter()
        .map(|id| TrackSpec {
            track_id: id.clone(),
        })
        .collect();

    let config = RecordConfig {
        take_dir: take_dir.clone(),
        tracks,
        channels: channel_count,
        sample_rate,
    };
    // Start the (device-free) pipeline first; if this fails nothing is open yet.
    let (sink, controller) = start_session(config)?;

    // Open the cpal input stream against the sink on its own thread. On failure
    // the controller (and its writer thread) is stopped so we leak nothing.
    let stream = match StreamHandle::spawn(device_name, sample_rate, channel_count as u16, sink) {
        Ok(s) => s,
        Err(e) => {
            let _ = controller.stop();
            return Err(e);
        }
    };

    let status = RecordingStatus {
        recording: true,
        captured_frames: controller.captured_frames() as f64,
        duration_ms: 0.0,
        dropped: controller.dropped() as f64,
        meters_dbfs: vec![f32::NEG_INFINITY; channel_count],
        writer_failed: controller.writer_failed(),
    };

    *recorder.session.lock().await = Some(RecorderSession {
        controller,
        stream,
        take_id,
        project_id,
        scast_dir,
        sample_rate,
        track_ids: session_track_ids,
        started_at,
    });

    Ok(status)
}

/// Stop the live recording, finalise the WAVs, and lay the captured audio onto
/// the timeline as a new take (one full-length region per non-empty track), then
/// return the refreshed timeline. A clean no-op-style error if nothing is live.
///
/// Teardown order matters: stop the stream FIRST (so the audio thread stops
/// pushing), THEN stop the writer thread (which does its final drain + finalise).
#[tauri::command]
pub async fn audio_record_stop(
    project: State<'_, ProjectState>,
    recorder: State<'_, RecorderControl>,
) -> AppResult<crate::project::model::TimelineSnapshot> {
    let session = recorder
        .session
        .lock()
        .await
        .take()
        .ok_or_else(|| AppError::Validation("no recording in progress".into()))?;

    let RecorderSession {
        controller,
        stream,
        take_id,
        project_id,
        scast_dir,
        sample_rate,
        track_ids,
        started_at,
    } = session;

    // Stop the device, then drain + finalise the writer for the per-track counts.
    stream.stop()?;
    let counts = controller.stop()?;

    let guard = project.current.lock().await;
    let op = current(&guard)?;
    // Guard against the project having been swapped while recording.
    if op.project_id != project_id {
        return Err(AppError::Validation(
            "the project changed while recording — the take was saved to disk but not placed"
                .into(),
        ));
    }
    let recorded = RecordedTake {
        take_id,
        sample_rate,
        started_at,
        track_ids,
        counts,
    };
    persist_recorded_take(&op.pool, &scast_dir, &project_id, &recorded).await
}

/// A finished recording's bookkeeping: the id its WAVs were written under, the
/// capture rate + wall-clock start, the armed track ids (channel order, also the
/// per-track WAV filenames), and the per-track captured sample counts. Bundled so
/// the persist seam below has a small, testable signature.
pub struct RecordedTake {
    /// Take id the live session wrote its WAVs under (also the take dir name).
    pub take_id: String,
    pub sample_rate: u32,
    /// Wall-clock start (ms) for the take row.
    pub started_at: f64,
    /// Armed track ids, channel order; each is its `<id>.wav` filename.
    pub track_ids: Vec<String>,
    /// Captured frames per track (index = channel), from the writer's finalise.
    pub counts: Vec<u64>,
}

/// Lay a finished recording onto the timeline: insert the take row under the id
/// the live session wrote its WAVs under, then place one full-length region per
/// track that captured a non-empty WAV (1:1 channel→track, like `import_takes`).
/// Split out from the command so it can be exercised against a throwaway pool +
/// temp `scast` dir with no device or Tauri state (see `tests/recording_transport.rs`).
pub async fn persist_recorded_take(
    pool: &sqlx::SqlitePool,
    scast_dir: &std::path::Path,
    project_id: &str,
    recorded: &RecordedTake,
) -> AppResult<crate::project::model::TimelineSnapshot> {
    let RecordedTake {
        take_id,
        sample_rate,
        started_at,
        track_ids,
        counts,
    } = recorded;

    // The take dir already holds one `<track_id>.wav` per capture track (written
    // live by the writer thread); derive each track's length from its count.
    let rate = (*sample_rate).max(1) as f64;
    let durations: Vec<f64> = counts.iter().map(|&n| n as f64 / rate * 1000.0).collect();
    let total_ms = durations.iter().cloned().fold(0.0_f64, f64::max);

    // Record the take row against the tracks that were armed for this take.
    store::add_take_with_id(pool, take_id, project_id, *started_at, total_ms, track_ids).await?;

    // Place one full-length region per track that captured a non-empty WAV.
    for (i, track_id) in track_ids.iter().enumerate() {
        let samples = counts.get(i).copied().unwrap_or(0);
        if samples == 0 {
            continue; // an armed track that received no audio — no region
        }
        let wav = scast::take_dir(scast_dir, take_id).join(format!("{track_id}.wav"));
        if !wav.exists() {
            continue;
        }
        store::add_region(
            pool,
            crate::project::model::Region {
                id: String::new(),
                take_id: take_id.to_string(),
                source_track_id: track_id.clone(),
                target_track_id: track_id.clone(),
                start_in_take_ms: 0.0,
                end_in_take_ms: durations[i],
                position_in_timeline_ms: 0.0,
                fade_in_ms: 5.0,
                fade_out_ms: 5.0,
                gain_adjust_db: 0.0,
            },
        )
        .await?;
    }

    store::load_timeline(pool, project_id).await
}

/// The recording transport state the UI polls (~60fps) to draw the record button,
/// the live take duration, the meters and any overrun warning. Returns the idle
/// status when no take is rolling — never errors.
#[tauri::command]
pub async fn audio_record_status(
    recorder: State<'_, RecorderControl>,
) -> AppResult<RecordingStatus> {
    let guard = recorder.session.lock().await;
    match guard.as_ref() {
        Some(s) => {
            let frames = s.controller.captured_frames();
            let channels = s.track_ids.len();
            let meters: Vec<f32> = (0..channels).map(|c| s.controller.meter_dbfs(c)).collect();
            Ok(RecordingStatus {
                recording: true,
                captured_frames: frames as f64,
                duration_ms: frames as f64 / s.sample_rate.max(1) as f64 * 1000.0,
                dropped: s.controller.dropped() as f64,
                meters_dbfs: meters,
                writer_failed: s.controller.writer_failed(),
            })
        }
        None => Ok(RecordingStatus::idle()),
    }
}

/// Tauri-managed handle to the live timeline-playback session.
///
/// Mirrors [`RecorderControl`]: the transport commands (`audio_play`,
/// `audio_pause`, `audio_seek`, `audio_playback_mute`) drive the
/// [`PlaybackController`] held here, and `audio_play_timeline` installs a fresh
/// controller (replacing/stopping any previous one). Until a session is started
/// the controller is absent and a transport call is a clean validation error the
/// UI can surface ("press play to start playback").
///
/// HARDWARE-UNVERIFIED: the controller's render thread + output ring are fully
/// tested without a device (see `audio::playback`), but the cpal **output**
/// stream that drains the ring to the speakers is the one hardware-dependent
/// piece, wired with Phase 2.2's transport — exactly as the recorder leaves its
/// input stream to the same phase.
#[derive(Default)]
pub struct PlaybackControl {
    pub(crate) session: Mutex<Option<PlaybackController>>,
}

impl PlaybackControl {
    /// Install a freshly-started controller, stopping any previous session first
    /// so we never leave a render thread running.
    async fn install(&self, controller: PlaybackController) {
        let mut guard = self.session.lock().await;
        if let Some(old) = guard.take() {
            let _ = old.stop();
        }
        *guard = Some(controller);
    }

    /// Run a closure against the live controller, erroring if none is active.
    async fn with<T>(&self, f: impl FnOnce(&PlaybackController) -> T) -> AppResult<T> {
        let guard = self.session.lock().await;
        let ctl = guard
            .as_ref()
            .ok_or_else(|| AppError::Validation("no active playback session".into()))?;
        Ok(f(ctl))
    }
}

/// The player's transport state — what the UI polls to draw the playhead and
/// transport buttons.
#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/PlaybackStatus.ts")]
pub struct PlaybackStatus {
    /// Whether the transport is rolling.
    pub playing: bool,
    /// Playhead position in milliseconds.
    pub position_ms: f64,
    /// Total timeline length in milliseconds.
    pub length_ms: f64,
}

/// Resolve the open project's timeline into per-track playback buffers: for each
/// track, assemble its placed regions (trimmed, gained, faded — the same
/// region-aware bake as export) into one mono timeline at the project's rate.
/// Each source WAV is decoded once and reused across the clips that reference it.
/// Pure-ish (no app state) so it can be exercised against a temp project; the
/// command below just supplies the open project's pool + folder (see
/// `tests/playback_resolve.rs`).
pub async fn resolve_playback_tracks(
    pool: &sqlx::SqlitePool,
    scast_dir: &std::path::Path,
    project_id: &str,
) -> AppResult<(Vec<PlaybackTrack>, u32)> {
    let project = store::load_project(pool).await?;
    let rate = project.sample_rate as u32;
    let tracks = store::list_tracks(pool, project_id).await?;
    let regions = store::list_project_regions(pool, project_id).await?;
    if regions.is_empty() {
        return Err(AppError::Validation(
            "nothing to play yet — import or record audio, then arrange clips on the timeline"
                .into(),
        ));
    }

    let soloed = tracks.iter().any(|t| t.solo);
    let scast_dir = scast_dir.to_path_buf();
    let mut cache: HashMap<PathBuf, Vec<f32>> = HashMap::new();
    let mut out: Vec<PlaybackTrack> = Vec::new();

    for t in &tracks {
        let mut clips: Vec<PlacedClip> = Vec::new();
        for r in regions.iter().filter(|r| r.target_track_id == t.id) {
            let wav_path =
                scast::take_dir(&scast_dir, &r.take_id).join(format!("{}.wav", r.source_track_id));
            if !wav_path.exists() {
                continue; // a region whose audio is missing — skip, don't abort
            }
            if !cache.contains_key(&wav_path) {
                let (samples, src_rate) =
                    render::read_wav_mono(&wav_path).map_err(AppError::Audio)?;
                if src_rate != rate {
                    return Err(AppError::Validation(format!(
                        "clip audio is {src_rate} Hz but the project is {rate} Hz — sample-rate conversion lands in a later phase"
                    )));
                }
                cache.insert(wav_path.clone(), samples);
            }
            let samples = render::render_region(
                &cache[&wav_path],
                rate,
                r.start_in_take_ms,
                r.end_in_take_ms,
                r.fade_in_ms,
                r.fade_out_ms,
                r.gain_adjust_db as f32,
            );
            clips.push(PlacedClip {
                position_ms: r.position_in_timeline_ms,
                samples,
            });
        }
        if clips.is_empty() {
            continue; // this track has no clips on the timeline
        }
        let mut timeline = render::assemble_timeline(&clips, rate);
        // Mute (or non-soloed when something is soloed) tracks play silent. Hard-
        // zeroing keeps the per-track mute bit free for live UI toggles during
        // playback (the timeline isn't rebuilt for a mute change).
        let silenced = t.mute || (soloed && !t.solo);
        if silenced {
            timeline.iter_mut().for_each(|s| *s = 0.0);
        }
        out.push(PlaybackTrack {
            timeline,
            gain_db: t.gain_db as f32,
        });
    }

    if out.is_empty() {
        return Err(AppError::Validation(
            "nothing audible to play — every track is muted or empty".into(),
        ));
    }
    Ok((out, rate))
}

/// Start playing the open project's timeline from its start. Resolves the
/// timeline (region-aware: trim/gain/fades baked per clip), spins up the render
/// thread, and installs the controller. Returns the initial transport state.
/// The heavy WAV decode + region bake run on the async-friendly store path; the
/// render thread itself does the realtime mixing off-thread.
#[tauri::command]
pub async fn audio_play_timeline(
    project: State<'_, ProjectState>,
    control: State<'_, PlaybackControl>,
) -> AppResult<PlaybackStatus> {
    let (tracks, rate) = {
        let guard = project.current.lock().await;
        let op = current(&guard)?;
        resolve_playback_tracks(&op.pool, &op.scast_dir, &op.project_id).await?
    };
    let controller = playback::start_playback(tracks, rate)?;
    controller.play();
    let status = PlaybackStatus {
        playing: controller.playing(),
        position_ms: controller.position_ms(),
        length_ms: controller.length() as f64 / rate.max(1) as f64 * 1000.0,
    };
    control.install(controller).await;
    Ok(status)
}

/// Resume the current playback session (after a pause). Errors if none is active.
#[tauri::command]
pub async fn audio_play(control: State<'_, PlaybackControl>) -> AppResult<()> {
    control.with(|c| c.play()).await
}

/// Pause the current playback session, holding the playhead. Errors if none is
/// active.
#[tauri::command]
pub async fn audio_pause(control: State<'_, PlaybackControl>) -> AppResult<()> {
    control.with(|c| c.pause()).await
}

/// Seek the playhead to a millisecond position (clamped to the timeline length).
#[tauri::command]
pub async fn audio_seek(control: State<'_, PlaybackControl>, position_ms: f64) -> AppResult<()> {
    control.with(|c| c.seek_ms(position_ms)).await
}

/// Mute/unmute a timeline track during playback (the timeline isn't rebuilt).
/// `track_idx` is the track's position in the resolved playback order.
#[tauri::command]
pub async fn audio_playback_mute(
    control: State<'_, PlaybackControl>,
    track_idx: usize,
    muted: bool,
) -> AppResult<()> {
    control.with(|c| c.set_mute(track_idx, muted)).await
}

/// The current transport state (the UI polls this ~60fps to draw the playhead).
#[tauri::command]
pub async fn audio_playback_status(
    control: State<'_, PlaybackControl>,
) -> AppResult<PlaybackStatus> {
    let guard = control.session.lock().await;
    match guard.as_ref() {
        Some(c) => Ok(PlaybackStatus {
            playing: c.playing(),
            position_ms: c.position_ms(),
            length_ms: c.length_ms(),
        }),
        None => Ok(PlaybackStatus {
            playing: false,
            position_ms: 0.0,
            length_ms: 0.0,
        }),
    }
}

/// Stop and tear down the current playback session (the render thread exits).
/// A no-op-friendly success when nothing is playing.
#[tauri::command]
pub async fn audio_stop_playback(control: State<'_, PlaybackControl>) -> AppResult<()> {
    let mut guard = control.session.lock().await;
    if let Some(c) = guard.take() {
        c.stop()?;
    }
    Ok(())
}

/// Build the [`LevelingSnapshot`] the AI auto-leveler reasons over: one row per
/// track with its current gain, clip count and measured integrated loudness /
/// true-peak. Loudness is measured from the track's raw region audio (no fader
/// gain applied) so the model sees the source levels, not a level we already
/// touched; a track with no on-disk audio simply reports `None` loudness.
///
/// Pure-ish (no app state) so it can be exercised against a temp project; the
/// command below supplies the open project's pool + folder. Each source WAV is
/// decoded once and cached across the regions that reference it.
async fn build_leveling_snapshot(
    pool: &sqlx::SqlitePool,
    scast_dir: &std::path::Path,
    project_id: &str,
) -> AppResult<LevelingSnapshot> {
    let project = store::load_project(pool).await?;
    let rate = project.sample_rate as u32;
    let tracks = store::list_tracks(pool, project_id).await?;
    let regions = store::list_project_regions(pool, project_id).await?;

    let mut cache: HashMap<PathBuf, Vec<f32>> = HashMap::new();
    let mut rows: Vec<LevelingTrack> = Vec::new();

    for t in &tracks {
        // Concatenate the track's raw region audio (un-gained) for measurement.
        let mut audio: Vec<f32> = Vec::new();
        let mut clip_count: u32 = 0;
        for r in regions.iter().filter(|r| r.target_track_id == t.id) {
            clip_count += 1;
            let wav_path =
                scast::take_dir(scast_dir, &r.take_id).join(format!("{}.wav", r.source_track_id));
            if !wav_path.exists() {
                continue;
            }
            if !cache.contains_key(&wav_path) {
                let (samples, src_rate) =
                    render::read_wav_mono(&wav_path).map_err(AppError::Audio)?;
                if src_rate != rate {
                    // Mixed rates are rejected elsewhere; here we just skip so the
                    // snapshot still builds rather than aborting the whole call.
                    continue;
                }
                cache.insert(wav_path.clone(), samples);
            }
            // Slice the region's range out of the source, in samples.
            let src = &cache[&wav_path];
            let start = ms_to_samples(r.start_in_take_ms, rate).min(src.len());
            let end = ms_to_samples(r.end_in_take_ms, rate).min(src.len());
            if end > start {
                audio.extend_from_slice(&src[start..end]);
            }
        }

        let measurement = if audio.is_empty() {
            None
        } else {
            // Mono measurement of the concatenated track audio.
            loudness::measure(&audio, 1, rate).ok()
        };

        rows.push(LevelingTrack {
            track_id: t.id.clone(),
            name: t.name.clone(),
            current_gain_db: t.gain_db,
            integrated_lufs: measurement.as_ref().and_then(|m| m.integrated_lufs),
            true_peak_dbtp: measurement.as_ref().and_then(|m| m.true_peak_dbtp),
            clip_count,
        });
    }

    // Default the target to the first platform target (Spotify, -16 LUFS); the
    // UI can re-run with a chosen target later if we surface that control.
    let target_lufs = loudness::loudness_targets()
        .first()
        .map(|t| t.integrated_lufs)
        .unwrap_or(-16.0);

    Ok(LevelingSnapshot {
        tracks: rows,
        target_lufs,
    })
}

/// Convert a millisecond position to a sample index at `rate`.
fn ms_to_samples(ms: f64, rate: u32) -> usize {
    ((ms / 1000.0) * rate as f64).max(0.0).round() as usize
}

/// AI auto-leveling (Phase 5.1, Pro). Snapshots the open project's tracks and
/// asks Claude (Anthropic Messages API) for per-track gain suggestions so a
/// multi-mic recording sits balanced before mastering.
///
/// The network call is opt-in and gated: it needs an `ANTHROPIC_API_KEY` (Free
/// tier / unconfigured returns a clean validation error). The blocking HTTP call
/// runs on `spawn_blocking` — AI is network I/O, never real-time, and never
/// touches the audio thread.
#[tauri::command]
pub async fn ai_auto_level(project: State<'_, ProjectState>) -> AppResult<LevelingResult> {
    let api_key = ai::anthropic_api_key().ok_or_else(|| {
        AppError::Validation(
            "AI auto-leveling needs an Anthropic API key (set ANTHROPIC_API_KEY). It's a Sunday Cast Pro feature.".into(),
        )
    })?;

    let snapshot = {
        let guard = project.current.lock().await;
        let op = current(&guard)?;
        build_leveling_snapshot(&op.pool, &op.scast_dir, &op.project_id).await?
    };

    if snapshot.tracks.is_empty() {
        return Err(AppError::Validation(
            "nothing to level yet — add or import tracks first".into(),
        ));
    }

    // The Anthropic call is blocking network I/O; keep it off the async runtime.
    tokio::task::spawn_blocking(move || {
        leveling::auto_level(&ReqwestTransport, &api_key, &snapshot)
            .map_err(|e| AppError::Internal(format!("AI auto-leveling failed: {e}")))
    })
    .await
    .map_err(|e| AppError::Internal(format!("auto-level task failed: {e}")))?
}

/// AI jingle generation (Phase 6, Pro) — the product's headline "wow" feature.
///
/// Takes a [`JingleSpec`] from the form and asks the music-generation wrapper
/// (an Edge Function fronting Suno, so the vendor key never reaches the client)
/// for a finished jingle, returning the generated audio's URL plus metadata for
/// the renderer to download and mix.
///
/// The network call is opt-in and gated: it needs a `SUNO_PROXY_URL` (Free tier
/// / unconfigured returns a clean validation error). The blocking HTTP call runs
/// on `spawn_blocking` — generation is network I/O, never real-time, and never
/// touches the audio thread.
#[tauri::command]
pub async fn ai_jingle_generate(spec: JingleSpec) -> AppResult<JingleResult> {
    let proxy_url = jingle::suno_proxy_url().ok_or_else(|| {
        AppError::Validation(
            "Jingle generation needs the music-generation service to be configured (set SUNO_PROXY_URL). It's a Sunday Cast Pro feature.".into(),
        )
    })?;

    // Re-validate server-side rather than trusting the form — a malformed
    // payload surfaces as a validation error before we spend a generation.
    let errors = jingle::validate_spec(&spec);
    if !errors.is_empty() {
        let joined = errors
            .iter()
            .map(|e| format!("{}: {}", e.field, e.message))
            .collect::<Vec<_>>()
            .join("; ");
        return Err(AppError::Validation(format!(
            "invalid jingle spec — {joined}"
        )));
    }

    // Generation is blocking network I/O; keep it off the async runtime.
    tokio::task::spawn_blocking(move || {
        jingle::generate_jingle(&ReqwestTransport, &proxy_url, &spec)
            .map_err(|e| AppError::Internal(format!("jingle generation failed: {e}")))
    })
    .await
    .map_err(|e| AppError::Internal(format!("jingle task failed: {e}")))?
}

/// AI show notes, chapters & clip suggestions (Phase 5.2, Pro).
///
/// Takes a transcript (from the SundayRec deep-link caption handoff, or pasted
/// into the edit panel) and asks Claude (Anthropic Messages API) for title
/// options, a Norwegian + English summary, timestamped chapters, tags, and a few
/// suggested highlight clips. The model only suggests; the parser sanitizes every
/// field against a strict schema (timestamps clamped into the take, chapters
/// ordered, lists bounded) so a bad reply can never corrupt the project. Accepted
/// chapters become ffmpeg chapter metadata on export.
///
/// The network call is opt-in and gated: it needs an `ANTHROPIC_API_KEY` (Free
/// tier / unconfigured returns a clean validation error so the panel shows "legg
/// til nøkkel for AI" and manual chapters keep working). The blocking HTTP call
/// runs on `spawn_blocking` — AI is network I/O, never real-time, and never
/// touches the audio thread.
#[tauri::command]
pub async fn ai_show_notes(input: ShowNotesInput) -> AppResult<ShowNotes> {
    let api_key = ai::anthropic_api_key().ok_or_else(|| {
        AppError::Validation(
            "AI show notes need an Anthropic API key (set ANTHROPIC_API_KEY). It's a Sunday Cast Pro feature.".into(),
        )
    })?;

    // Re-check server-side rather than trusting the renderer — an empty
    // transcript surfaces as a validation error before we spend a request.
    if input.transcript.trim().is_empty() {
        return Err(AppError::Validation(
            "no transcript to summarise — import captions or paste a transcript first".into(),
        ));
    }

    // The Anthropic call is blocking network I/O; keep it off the async runtime.
    tokio::task::spawn_blocking(move || {
        shownotes::generate_show_notes(&ReqwestTransport, &api_key, &input)
            .map_err(|e| AppError::Internal(format!("AI show notes failed: {e}")))
    })
    .await
    .map_err(|e| AppError::Internal(format!("show-notes task failed: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_command_succeeds() {
        // Environment-independent: must return a list (possibly empty) and a host.
        let list = audio_devices().expect("audio_devices ok");
        assert!(!list.host.is_empty());
    }

    #[test]
    fn test_tone_command_writes_a_file() {
        let res = audio_record_test_tone().expect("test tone ok");
        assert!(std::path::Path::new(&res.path).exists());
        assert_eq!(res.sample_rate, 48_000);
        assert_eq!(res.frequency, 440.0);
        assert!(res.bytes > 44, "more than just a header");
    }

    #[tokio::test]
    async fn monitor_command_errors_with_no_active_recording() {
        // No take rolling → toggling monitoring is a clean validation error,
        // never a panic or a silent no-op.
        let recorder = RecorderControl::default();
        let err = recorder
            .send_command(RecorderCommand::SetMonitoring(true))
            .await
            .unwrap_err();
        assert_eq!(err.code(), AppError::Validation(String::new()).code());
        assert!(!recorder.is_recording().await);
    }

    #[tokio::test]
    async fn record_status_is_idle_with_no_active_recording() {
        // The status poll never errors; with nothing rolling it reports idle.
        let recorder = RecorderControl::default();
        let guard = recorder.session.lock().await;
        assert!(guard.is_none());
        let status = RecordingStatus::idle();
        assert!(!status.recording);
        assert_eq!(status.captured_frames, 0.0);
        assert!(status.meters_dbfs.is_empty());
    }

    #[tokio::test]
    async fn playback_transport_errors_with_no_active_session() {
        // No session installed → every transport call is a clean validation
        // error, never a panic or a silent no-op.
        let control = PlaybackControl::default();
        let err = control.with(|c| c.playing()).await.unwrap_err();
        assert_eq!(err.code(), AppError::Validation(String::new()).code());
    }

    #[tokio::test]
    async fn stop_playback_is_a_no_op_when_nothing_is_playing() {
        let control = PlaybackControl::default();
        // Tearing down with no session is a clean success.
        audio_stop_playback_inner(&control).await.unwrap();
        // Status reports the stopped/empty default.
        let guard = control.session.lock().await;
        assert!(guard.is_none());
    }

    /// Test shim for [`audio_stop_playback`] without a Tauri `State` wrapper.
    async fn audio_stop_playback_inner(control: &PlaybackControl) -> AppResult<()> {
        let mut guard = control.session.lock().await;
        if let Some(c) = guard.take() {
            c.stop()?;
        }
        Ok(())
    }

    #[tokio::test]
    async fn install_replaces_and_stops_the_previous_session() {
        use crate::audio::playback::{start_playback, PlaybackTrack};

        let control = PlaybackControl::default();
        let first = start_playback(
            vec![PlaybackTrack {
                timeline: vec![0.1; 480],
                gain_db: 0.0,
            }],
            48_000,
        )
        .unwrap();
        control.install(first).await;
        assert!(control.with(|c| c.length()).await.unwrap() == 480);

        // Installing a second session stops the first and takes over.
        let second = start_playback(
            vec![PlaybackTrack {
                timeline: vec![0.1; 960],
                gain_db: 0.0,
            }],
            48_000,
        )
        .unwrap();
        control.install(second).await;
        assert_eq!(control.with(|c| c.length()).await.unwrap(), 960);
    }
}
