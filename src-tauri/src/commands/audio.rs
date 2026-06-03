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

use crate::audio::playback::{self, PlaybackController, PlaybackTrack};
use crate::audio::recorder::{CommandTx, RecorderCommand};
use crate::audio::{devices, settings, tone};
use crate::commands::project::{current, ProjectState};
use crate::error::{AppError, AppResult};
use crate::export::render::{self, PlacedClip};
use crate::project::{scast, store};

/// Tauri-managed handle to the live recording session's control queue.
///
/// The monitor commands (`audio_set_monitoring`, `audio_set_monitor_mute`) flip
/// state on the audio thread by enqueueing into this lock-free queue. A session
/// installs its `CommandTx` here when it starts (Phase 2.2's live transport) and
/// clears it on stop; until then the queue is absent and a monitor toggle is a
/// no-op error the UI can surface ("start a recording to monitor").
#[derive(Default)]
pub struct MonitorControl {
    pub(crate) commands: Mutex<Option<CommandTx>>,
}

impl MonitorControl {
    /// Register the live session's command sender (called by the transport when
    /// a session starts). Replacing any previous sender drops the old one.
    pub async fn attach(&self, tx: CommandTx) {
        *self.commands.lock().await = Some(tx);
    }

    /// Drop the command sender when the session ends.
    pub async fn detach(&self) {
        *self.commands.lock().await = None;
    }

    /// Enqueue a control command, erroring if no session is live or the queue is
    /// momentarily full.
    async fn send(&self, cmd: RecorderCommand) -> AppResult<()> {
        let mut guard = self.commands.lock().await;
        let tx = guard
            .as_mut()
            .ok_or_else(|| AppError::Validation("no active recording session to monitor".into()))?;
        if tx.send(cmd) {
            Ok(())
        } else {
            Err(AppError::Audio("monitor command queue full; retry".into()))
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
/// `SetMonitoring` for the audio thread, which starts/stops feeding the mono
/// monitor mix to the output callback. Errors if no session is recording.
#[tauri::command]
pub async fn audio_set_monitoring(
    control: State<'_, MonitorControl>,
    enabled: bool,
) -> AppResult<()> {
    control.send(RecorderCommand::SetMonitoring(enabled)).await
}

/// Mute/unmute one track in the monitor mix without affecting capture (Phase
/// 1.3). `track_idx` is the input-channel index. Errors if no session is live.
#[tauri::command]
pub async fn audio_set_monitor_mute(
    control: State<'_, MonitorControl>,
    track_idx: usize,
    muted: bool,
) -> AppResult<()> {
    control
        .send(RecorderCommand::SetMute {
            track: track_idx,
            muted,
        })
        .await
}

/// Tauri-managed handle to the live timeline-playback session.
///
/// Mirrors [`MonitorControl`]: the transport commands (`audio_play`,
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
    async fn monitor_command_errors_with_no_active_session() {
        // No session attached → toggling monitoring is a clean validation error,
        // never a panic or a silent no-op.
        let control = MonitorControl::default();
        let err = control
            .send(RecorderCommand::SetMonitoring(true))
            .await
            .unwrap_err();
        assert_eq!(err.code(), AppError::Validation(String::new()).code());
    }

    #[tokio::test]
    async fn attached_session_receives_monitor_commands() {
        use crate::audio::recorder::command_channel;

        let (tx, mut rx) = command_channel(8);
        let control = MonitorControl::default();
        control.attach(tx).await;

        control
            .send(RecorderCommand::SetMonitoring(true))
            .await
            .expect("monitoring enqueues");
        control
            .send(RecorderCommand::SetMute {
                track: 1,
                muted: true,
            })
            .await
            .expect("mute enqueues");

        assert_eq!(rx.try_recv(), Some(RecorderCommand::SetMonitoring(true)));
        assert_eq!(
            rx.try_recv(),
            Some(RecorderCommand::SetMute {
                track: 1,
                muted: true
            })
        );

        // After detach, commands error again.
        control.detach().await;
        assert!(control
            .send(RecorderCommand::SetMonitoring(false))
            .await
            .is_err());
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
