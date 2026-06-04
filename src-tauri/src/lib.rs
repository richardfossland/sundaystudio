//! SundayStudio main library — Tauri runtime entry point.
//!
//! Phase 0.1 wires up the bare bridge plus the first audio smoke test: tracing
//! and three IPC commands —
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

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::project::ProjectState::default())
        .manage(commands::audio::RecorderControl::default())
        .manage(commands::audio::PlaybackControl::default());

    // The `sundaystudio://import` deep link is desktop-only (the scheme itself
    // is registered by the bundler from `tauri.conf.json` → plugins.deep-link).
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_deep_link::init());
    }

    builder
        .setup(|app| {
            tracing::info!("SundayStudio backend ready");
            // Sunday-link RECEIVER: forward an inbound `sundaystudio://import?…`
            // URL (SundayRec → Studio handoff) to the renderer, which validates
            // it via `deeplink_parse_import` and drives the take import.
            // HARDWARE-UNVERIFIED: compiles + wired, but the OS-level scheme
            // dispatch has not been exercised on a real macOS/Windows install.
            #[cfg(desktop)]
            {
                use tauri::Emitter;
                use tauri_plugin_deep_link::DeepLinkExt;
                let handle = app.handle().clone();
                app.deep_link().on_open_url(move |event| {
                    for url in event.urls() {
                        let _ = handle.emit("deep-link://import", url.to_string());
                    }
                });
            }
            let _ = app;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app::app_info,
            commands::audio::audio_devices,
            commands::audio::audio_record_test_tone,
            commands::audio::audio_get_settings,
            commands::audio::audio_set_settings,
            commands::audio::audio_latency_estimate,
            commands::audio::audio_set_monitoring,
            commands::audio::audio_set_monitor_mute,
            commands::audio::audio_record_start,
            commands::audio::audio_record_stop,
            commands::audio::audio_record_status,
            commands::audio::audio_play_timeline,
            commands::audio::audio_play,
            commands::audio::audio_pause,
            commands::audio::audio_seek,
            commands::audio::audio_playback_mute,
            commands::audio::audio_playback_status,
            commands::audio::audio_stop_playback,
            commands::audio::ai_auto_level,
            commands::audio::ai_jingle_generate,
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
            // Phase 2.1 registry CRUD
            commands::project::project_new,
            commands::project::project_save,
            commands::project::project_load,
            commands::project::project_list,
            commands::project::project_delete,
            commands::dsp::dsp_presets,
            commands::dsp::dsp_loudness_targets,
            commands::dsp::dsp_master_presets,
            commands::dsp::dsp_analyze_file,
            commands::export::export_presets,
            commands::export::export_render,
            commands::edit::project_timeline,
            commands::edit::audio_peaks,
            commands::edit::analyze_silence,
            commands::edit::region_add,
            commands::edit::region_create,
            commands::edit::region_update,
            commands::edit::region_delete,
            commands::edit::take_import,
            commands::deeplink::deeplink_parse_import,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
