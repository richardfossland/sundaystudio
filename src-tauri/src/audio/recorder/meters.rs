//! Level metering — the lock-free bridge from the audio thread to the UI.
//!
//! The audio callback computes a per-channel peak for each block and stores it
//! into an atomic slot; the UI thread reads (and resets) those slots at ~60fps.
//! No locks, no allocation on the audio side — just an atomic store per block.
//!
//! Peaks are non-negative, so we can keep a peak-hold-since-last-read using
//! `fetch_max` on the f32 bit pattern: for non-negative floats the bitwise
//! order matches the numeric order.

use std::sync::atomic::{AtomicU32, Ordering};

/// Largest absolute sample in a block (linear, 0.0..=~1.0). Real-time safe:
/// no allocation, bounded work.
pub fn block_peak(samples: &[f32]) -> f32 {
    samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max)
}

/// Convert a linear peak (0.0..1.0) to dBFS. Zero/at-floor reads as -inf.
pub fn peak_to_dbfs(peak: f32) -> f32 {
    if peak <= 0.0 {
        f32::NEG_INFINITY
    } else {
        20.0 * peak.log10()
    }
}

/// One atomic peak slot per channel, holding the max since the last read.
pub struct PeakMeters {
    slots: Vec<AtomicU32>,
}

impl PeakMeters {
    pub fn new(channels: usize) -> Self {
        Self {
            slots: (0..channels).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    pub fn channels(&self) -> usize {
        self.slots.len()
    }

    /// Record a peak for a channel, keeping the max since the last `take`.
    /// Called from the audio thread once per block. Out-of-range channels and
    /// non-finite/negative peaks are ignored (defensive; never panics on the
    /// real-time path).
    pub fn observe(&self, channel: usize, peak: f32) {
        if !peak.is_finite() || peak < 0.0 {
            return;
        }
        if let Some(slot) = self.slots.get(channel) {
            slot.fetch_max(peak.to_bits(), Ordering::AcqRel);
        }
    }

    /// Read and reset a channel's held peak (linear). The UI polls this.
    pub fn take(&self, channel: usize) -> f32 {
        match self.slots.get(channel) {
            Some(slot) => f32::from_bits(slot.swap(0, Ordering::AcqRel)),
            None => 0.0,
        }
    }

    /// Read and reset a channel's held peak in dBFS (UI convenience).
    pub fn take_dbfs(&self, channel: usize) -> f32 {
        peak_to_dbfs(self.take(channel))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_peak_is_max_abs() {
        assert_eq!(block_peak(&[0.1, -0.7, 0.3]), 0.7);
        assert_eq!(block_peak(&[]), 0.0);
    }

    #[test]
    fn dbfs_mapping() {
        assert!((peak_to_dbfs(1.0) - 0.0).abs() < 1e-4);
        assert!((peak_to_dbfs(0.5) + 6.0206).abs() < 1e-3);
        assert_eq!(peak_to_dbfs(0.0), f32::NEG_INFINITY);
    }

    #[test]
    fn meters_hold_max_then_reset_on_take() {
        let m = PeakMeters::new(2);
        m.observe(0, 0.3);
        m.observe(0, 0.8);
        m.observe(0, 0.5); // max so far is 0.8
        m.observe(1, 0.25);

        assert!((m.take(0) - 0.8).abs() < 1e-6);
        assert!((m.take(1) - 0.25).abs() < 1e-6);
        // After take, slots reset to 0.
        assert_eq!(m.take(0), 0.0);
    }

    #[test]
    fn meters_ignore_bad_input_and_oob_channels() {
        let m = PeakMeters::new(1);
        m.observe(0, f32::NAN);
        m.observe(0, -1.0);
        m.observe(5, 0.9); // out of range
        assert_eq!(m.take(0), 0.0);
        assert_eq!(m.take(5), 0.0);
        assert_eq!(m.channels(), 1);
    }
}
