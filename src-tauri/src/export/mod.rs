//! Encoding + platform export (Phase 7).
//!
//! Phase 7.1a lands the bounce: [`render`] mixes the project's per-track WAVs,
//! runs the Phase 4.2 master chain, and loudness-normalises to a platform target,
//! writing a 24-bit master WAV. [`format`] holds the export formats and the
//! platform-ready presets (Spotify −14, Apple −16, …).
//!
//! Phase 7.1b adds the encode step: [`encode`] builds the deterministic ffmpeg
//! argument vector for the master-WAV re-encode (pure, unit-tested offline) and
//! spawns the bundled sidecar to produce MP3/AAC/FLAC, falling back to the master
//! WAV when ffmpeg is unavailable. Still to come: ID3v2 metadata + embedded
//! chapters, and RSS / direct-upload helpers (7.2). Native WAV output is complete
//! and clip-safe today; the lossy formats carry `requires_encoder = true`.

pub mod encode;
pub mod fade;
pub mod format;
pub mod render;
