//! Audio commands — the Phase 0.1 smoke test of the highest-risk subsystem.
//!
//! `audio_devices` enumerates the system's input/output devices (proves cpal
//! links and talks to CoreAudio/WASAPI). `audio_record_test_tone` writes a
//! 1-second sine WAV to disk (proves our WAV writing path works). Together they
//! exercise the device layer and the file layer before Phase 1 builds the
//! real-time recording engine on top.

use tauri::{AppHandle, Manager};

use crate::audio::{devices, settings, tone};
use crate::error::{AppError, AppResult};

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
}
