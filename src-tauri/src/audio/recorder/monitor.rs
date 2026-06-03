//! Low-latency monitoring mixer (Phase 1.3).
//!
//! While capture writes every armed channel to its own WAV, the player needs to
//! *hear* what's being recorded in real time. The monitor path sums all armed
//! input channels into one mono block, applies a per-track soft mute, and pushes
//! the result into a dedicated ring that the cpal **output** callback drains to
//! the headphones (see `stream.rs` / docs/ARCHITECTURE.md, "Monitor ring").
//!
//! Split by testability, like the rest of the recorder:
//! - `mix_monitor_block` — the pure DSP: interleaved input → mono mix, honouring
//!   the mute mask. No state, no allocation, no device; the integration test
//!   drives it directly and asserts the mix math (sum of armed tracks, mute is
//!   linear, length matches the input block).
//! - `MonitorState` — the lock-free control surface the audio thread reads each
//!   block: an `enabled` flag and a per-track mute bitmask, both atomics so the
//!   UI→audio command queue can flip them without locking the real-time path.
//!
//! "Soft" mute means the muted track simply contributes 0.0 to the sum (a clean
//! gate, no click suppression yet — a smoothed ramp is a v2 refinement, noted in
//! DECISIONS.md). An optional monitor DSP chain (the same `VoiceChain` used for
//! recording) is a v2 first pass too: today the monitor mixes dry.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// The most input channels we track a mute bit for. Eight is the product
/// ceiling (8 simultaneous mics, see CLAUDE.md), so a `u64` bitmask is ample and
/// keeps the mute set lock-free and allocation-free on the audio thread.
pub const MAX_MONITOR_TRACKS: usize = 64;

/// Lock-free control surface for the monitor mixer, shared between the UI
/// (command queue) and the audio callback. Cheap to read every block.
#[derive(Debug)]
pub struct MonitorState {
    /// Whether the monitor ring is being fed at all.
    enabled: AtomicBool,
    /// One bit per track: set = muted in the monitor mix. Bit `i` ↔ track `i`.
    mute_mask: AtomicU64,
}

impl Default for MonitorState {
    fn default() -> Self {
        Self {
            // Monitoring starts off: the player opts in (and we never feed an
            // output device that may not exist until they do).
            enabled: AtomicBool::new(false),
            mute_mask: AtomicU64::new(0),
        }
    }
}

impl MonitorState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the monitor mix currently being produced?
    #[inline]
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Turn software monitoring on/off (UI side; flips the flag the audio
    /// callback checks each block).
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Release);
    }

    /// Is `track` muted in the monitor mix? Out-of-range tracks read unmuted.
    #[inline]
    pub fn is_muted(&self, track: usize) -> bool {
        if track >= MAX_MONITOR_TRACKS {
            return false;
        }
        self.mute_mask.load(Ordering::Acquire) & (1u64 << track) != 0
    }

    /// Mute/unmute a track in the monitor mix (capture is unaffected). A
    /// `fetch_and`/`fetch_or` keeps this a single lock-free RMW.
    pub fn set_muted(&self, track: usize, muted: bool) {
        if track >= MAX_MONITOR_TRACKS {
            return;
        }
        let bit = 1u64 << track;
        if muted {
            self.mute_mask.fetch_or(bit, Ordering::AcqRel);
        } else {
            self.mute_mask.fetch_and(!bit, Ordering::AcqRel);
        }
    }

    /// The raw mute bitmask (for tests / diagnostics).
    pub fn mute_mask(&self) -> u64 {
        self.mute_mask.load(Ordering::Acquire)
    }
}

/// Sum one block of interleaved input into a mono monitor block, skipping muted
/// channels. Real-time safe: writes into the caller-owned `out` (cleared and
/// resized first by `prepare_out`), no allocation, bounded work.
///
/// `data` is `frames × channels` interleaved (exactly what cpal hands the input
/// callback). `out` ends up with one sample per frame: the sum of every armed,
/// unmuted channel for that frame. Summing (not averaging) matches a hardware
/// mixer bus — gain staging is the player's job via per-track levels, the same
/// way the recorder treats each channel independently.
pub fn mix_monitor_block(data: &[f32], channels: usize, mute: &MonitorState, out: &mut Vec<f32>) {
    out.clear();
    if channels == 0 {
        return;
    }
    let frames = data.len() / channels;
    out.reserve(frames);

    // Precompute the gain per channel once (1.0 or 0.0) so the inner loop is a
    // branch-free multiply-accumulate.
    let mut gains = [0.0_f32; MAX_MONITOR_TRACKS];
    let active = channels.min(MAX_MONITOR_TRACKS);
    for (c, g) in gains.iter_mut().enumerate().take(active) {
        *g = if mute.is_muted(c) { 0.0 } else { 1.0 };
    }

    for f in 0..frames {
        let base = f * channels;
        let mut sum = 0.0_f32;
        for (c, &g) in gains.iter().enumerate().take(active) {
            sum += data[base + c] * g;
        }
        out.push(sum);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_state_defaults_off_and_unmuted() {
        let s = MonitorState::new();
        assert!(!s.enabled());
        assert_eq!(s.mute_mask(), 0);
        assert!(!s.is_muted(0));
    }

    #[test]
    fn toggling_enabled_and_mute_is_visible() {
        let s = MonitorState::new();
        s.set_enabled(true);
        assert!(s.enabled());

        s.set_muted(2, true);
        assert!(s.is_muted(2));
        assert!(!s.is_muted(1));
        assert_eq!(s.mute_mask(), 0b100);

        s.set_muted(2, false);
        assert!(!s.is_muted(2));
        assert_eq!(s.mute_mask(), 0);
    }

    #[test]
    fn out_of_range_track_is_ignored() {
        let s = MonitorState::new();
        s.set_muted(MAX_MONITOR_TRACKS, true); // no-op, must not panic
        assert_eq!(s.mute_mask(), 0);
        assert!(!s.is_muted(MAX_MONITOR_TRACKS + 10));
    }

    #[test]
    fn mix_sums_all_armed_channels_per_frame() {
        // 2 frames, 4 channels: ch0=0.1 ch1=0.2 ch2=0.3 ch3=0.4 each frame.
        let data = [0.1, 0.2, 0.3, 0.4, 0.1, 0.2, 0.3, 0.4];
        let s = MonitorState::new();
        let mut out = Vec::new();
        mix_monitor_block(&data, 4, &s, &mut out);
        assert_eq!(out.len(), 2, "one mono sample per frame");
        for v in out {
            assert!((v - 1.0).abs() < 1e-6, "sum of 0.1+0.2+0.3+0.4 = 1.0");
        }
    }

    #[test]
    fn mute_zeroes_one_track_in_the_sum() {
        // Mute track 2: its 0.3 drops out, leaving 0.1+0.2+0.4 = 0.7.
        let data = [0.1, 0.2, 0.3, 0.4];
        let s = MonitorState::new();
        s.set_muted(2, true);
        let mut out = Vec::new();
        mix_monitor_block(&data, 4, &s, &mut out);
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0.7).abs() < 1e-6, "got {}", out[0]);
    }

    #[test]
    fn mute_math_is_linear_n_tracks_minus_one() {
        // N identical tracks at 1.0 → mix is N; muting one → N-1.
        let n = 8;
        let frame: Vec<f32> = vec![1.0; n];
        let s = MonitorState::new();
        let mut out = Vec::new();

        mix_monitor_block(&frame, n, &s, &mut out);
        assert!((out[0] - n as f32).abs() < 1e-6);

        s.set_muted(3, true);
        mix_monitor_block(&frame, n, &s, &mut out);
        assert!((out[0] - (n - 1) as f32).abs() < 1e-6);
    }

    #[test]
    fn zero_channels_yields_empty_mix() {
        let s = MonitorState::new();
        let mut out = vec![1.0, 2.0]; // must be cleared
        mix_monitor_block(&[0.1, 0.2], 0, &s, &mut out);
        assert!(out.is_empty());
    }
}
