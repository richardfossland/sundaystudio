//! Encoding + platform export (Phase 7).
//!
//! Phase 7.1a lands the bounce: [`render`] mixes the project's per-track WAVs,
//! runs the Phase 4.2 master chain, and loudness-normalises to a platform target,
//! writing a 24-bit master WAV. [`format`] holds the export formats and the
//! platform-ready presets (Spotify −14, Apple −16, …).
//!
//! Still to come: MP3/AAC/FLAC encoding via a bundled ffmpeg sidecar, ID3v2
//! metadata + embedded chapters (7.1b), and RSS / direct-upload helpers (7.2).
//! Native WAV output is complete and clip-safe today; the lossy formats carry
//! `requires_encoder = true` until the sidecar is wired.

pub mod format;
pub mod render;
