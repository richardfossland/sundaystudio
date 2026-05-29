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
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::project::ProjectState::default())
        .setup(|_app| {
            tracing::info!("SundayStudio backend ready");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_devices,
            commands::audio::audio_record_test_tone,
            commands::audio::audio_get_settings,
            commands::audio::audio_set_settings,
            commands::audio::audio_latency_estimate,
            commands::project::project_create,
            commands::project::project_templates,
            commands::project::project_create_from_template,
            commands::project::project_open,
            commands::project::project_recent,
            commands::project::project_snapshot,
            commands::project::project_rename,
            commands::project::project_backup,
            commands::project::track_add,
            commands::project::track_update,
            commands::project::track_delete,
            commands::project::marker_add,
            commands::project::marker_delete,
            commands::dsp::dsp_presets,
            commands::dsp::dsp_loudness_targets,
            commands::dsp::dsp_master_presets,
            commands::dsp::dsp_analyze_file,
            commands::export::export_presets,
            commands::export::export_render,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
