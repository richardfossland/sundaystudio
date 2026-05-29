//! Audio commands — the Phase 0.1 smoke test of the highest-risk subsystem.
//!
//! `audio_devices` enumerates the system's input/output devices (proves cpal
//! links and talks to CoreAudio/WASAPI). `audio_record_test_tone` writes a
//! 1-second sine WAV to disk (proves our WAV writing path works). Together they
//! exercise the device layer and the file layer before Phase 1 builds the
//! real-time recording engine on top.

use crate::audio::{devices, tone};
use crate::error::AppResult;

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
