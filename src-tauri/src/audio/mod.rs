//! The audio engine.
//!
//! Phase 0.1 ships only the two pieces that smoke-test the riskiest part of the
//! stack end to end:
//!   - `devices`  enumerate input/output devices and their capabilities (cpal)
//!   - `tone`     synthesise + write a 1-second sine WAV to disk (hound)
//!
//! Phase 1 grows this into the real engine: a real-time `recorder` driven by a
//! cpal callback feeding lock-free ring buffers, a `writer` thread draining them
//! to per-track WAVs, a `monitor` path for low-latency headphone mixing, and a
//! `mixer`. Those modules deliberately do not exist yet — the foundation is
//! proven first (see docs/ARCHITECTURE.md).

pub mod devices;
pub mod peaks;
pub mod playback;
pub mod recorder;
pub mod settings;
pub mod tone;
