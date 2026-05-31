//! Tauri command handlers.
//!
//! Commands are the thin IPC layer the renderer calls via `invoke()`. They
//! delegate to the domain modules (`audio`, later `project` / `export` / `ai`)
//! and return `Result<T, AppError>`. Naming convention: `entity_verb`
//! (e.g. `app_info`, `audio_devices`, `audio_record_test_tone`).

pub mod app;
pub mod audio;
pub mod deeplink;
pub mod dsp;
pub mod edit;
pub mod export;
pub mod project;
