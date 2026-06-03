//! Audio commands — the Phase 0.1 smoke test of the highest-risk subsystem.
//!
//! `audio_devices` enumerates the system's input/output devices (proves cpal
//! links and talks to CoreAudio/WASAPI). `audio_record_test_tone` writes a
//! 1-second sine WAV to disk (proves our WAV writing path works). Together they
//! exercise the device layer and the file layer before Phase 1 builds the
//! real-time recording engine on top.

use tauri::{AppHandle, Manager, State};
use tokio::sync::Mutex;

use crate::audio::recorder::{CommandTx, RecorderCommand};
use crate::audio::{devices, settings, tone};
use crate::error::{AppError, AppResult};

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
}
