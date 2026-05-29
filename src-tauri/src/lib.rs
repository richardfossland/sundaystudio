//! SundayStudio main library — Tauri runtime entry point.
//!
//! Phase 0.1 wires up the bare bridge plus the first audio smoke test: tracing,
//! the opener plugin, and three IPC commands —
//!   - `app_info`              proves Rust ↔ React works
//!   - `audio_devices`         enumerates input/output devices via cpal
//!   - `audio_record_test_tone` writes a 1-second sine WAV to disk via hound
//!
//! These two audio commands exist to prove the highest-risk part of the stack
//! links and runs on both CoreAudio (macOS) and WASAPI (Windows) *before* we
//! build the real-time recording engine in Phase 1.
//!
//! Module map (most are placeholders until their phase):
//!   audio    real-time engine — devices (0.1), recorder/mixer/monitor (Phase 1)
//!   dsp      built-in effects — gate, EQ, compressor, limiter (Phase 4)
//!   project  project file format + SQLite persistence (Phase 2.1)
//!   export   encoding + platform export via ffmpeg sidecar (Phase 7)
//!   ai       Anthropic / Suno wrappers (Phase 5/6)
//! Command implementations live in `commands::*` — this file only registers them.

pub mod ai;
pub mod audio;
pub mod commands;
pub mod dsp;
pub mod error;
pub mod export;
pub mod project;
pub mod services;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_target(false)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|_app| {
            tracing::info!("SundayStudio backend ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_devices,
            commands::audio::audio_record_test_tone,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
