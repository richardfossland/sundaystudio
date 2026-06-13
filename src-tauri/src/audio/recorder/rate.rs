//! Sample-rate selection for the capture device, separated from the cpal stream
//! so the decision logic is pure and unit-testable WITHOUT audio hardware.
//!
//! The recorder records the *project* at a fixed rate (e.g. 48 kHz), but the
//! input interface may not be able to open at that rate. The old code queried
//! `device.default_input_config()` only to read the sample format, then forced
//! the project rate into the `StreamConfig` verbatim — with no check that the
//! device could actually run at it. On an interface locked to 44.1 kHz that
//! either errors at `build_input_stream` or, worse on some backends, opens at
//! the device's own rate and silently records audio that plays back at the
//! wrong speed/pitch.
//!
//! This module decides the rate the device is actually opened at:
//!   1. If the requested (project) rate falls inside one of the device's
//!      supported ranges, use it verbatim (the common, no-resample case).
//!   2. Otherwise clamp to the nearest reachable rate among the supported
//!      ranges' endpoints.
//!   3. If the device advertises no usable ranges at all, fall back to a
//!      caller-supplied default (the device's `default_input_config` rate).
//!
//! When the chosen device rate differs from the project rate, `stream.rs`
//! resamples the captured blocks up/down to the project rate via `rubato`
//! before they reach the writer, so what lands on disk is always at the
//! project rate regardless of what the hardware could do.

/// An inclusive sample-rate range a device range advertises (Hz). Mirrors the
/// `min`/`max` of a `cpal::SupportedStreamConfigRange` but is a plain value so
/// the selection logic below has no cpal dependency and is trivially testable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateRange {
    pub min: u32,
    pub max: u32,
}

impl RateRange {
    pub fn new(min: u32, max: u32) -> Self {
        // Defensive: keep min <= max even if a backend reports them swapped.
        if min <= max {
            RateRange { min, max }
        } else {
            RateRange { min: max, max: min }
        }
    }

    /// Does this range cover `rate`?
    fn contains(&self, rate: u32) -> bool {
        rate >= self.min && rate <= self.max
    }

    /// The reachable rate in this range closest to `rate` (clamped to bounds).
    fn clamp(&self, rate: u32) -> u32 {
        rate.clamp(self.min, self.max)
    }
}

/// Pick the sample rate the device should actually be opened at, given the
/// requested (project) rate, the device's supported input ranges, and a
/// `default_rate` fallback (the device's reported default config rate).
///
/// - Returns `requested` unchanged if any supported range covers it.
/// - Otherwise returns the supported rate nearest to `requested` (clamped to
///   the closest range's bounds), preferring the lower rate on an exact tie so
///   the choice is deterministic.
/// - If `ranges` is empty (device reports no usable input configs), returns
///   `default_rate`.
pub fn select_capture_rate(requested: u32, ranges: &[RateRange], default_rate: u32) -> u32 {
    if ranges.is_empty() {
        return default_rate;
    }

    // Common case: the device can run at exactly what the project wants.
    if ranges.iter().any(|r| r.contains(requested)) {
        return requested;
    }

    // Unsupported: clamp into each range and keep the candidate whose distance
    // to `requested` is smallest. Ties resolve to the lower rate for
    // determinism (so tests are stable and the result never depends on range
    // ordering).
    let mut best = ranges[0].clamp(requested);
    let mut best_dist = best.abs_diff(requested);
    for r in &ranges[1..] {
        let cand = r.clamp(requested);
        let dist = cand.abs_diff(requested);
        if dist < best_dist || (dist == best_dist && cand < best) {
            best = cand;
            best_dist = dist;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requested_rate_used_verbatim_when_supported() {
        // A device advertising a continuous 44.1–96 kHz range covers 48 kHz.
        let ranges = [RateRange::new(44_100, 96_000)];
        assert_eq!(select_capture_rate(48_000, &ranges, 44_100), 48_000);
    }

    #[test]
    fn requested_rate_used_when_it_equals_a_range_bound() {
        let ranges = [RateRange::new(48_000, 48_000)];
        assert_eq!(select_capture_rate(48_000, &ranges, 44_100), 48_000);
    }

    #[test]
    fn clamps_up_to_the_minimum_when_request_is_below_every_range() {
        // Interface fixed at 48 kHz; project wants 44.1 → clamp up to 48 kHz.
        let ranges = [RateRange::new(48_000, 48_000)];
        assert_eq!(select_capture_rate(44_100, &ranges, 96_000), 48_000);
    }

    #[test]
    fn clamps_down_to_the_maximum_when_request_is_above_every_range() {
        // Interface fixed at 44.1 kHz; project wants 96 → clamp down to 44.1.
        let ranges = [RateRange::new(44_100, 44_100)];
        assert_eq!(select_capture_rate(96_000, &ranges, 48_000), 44_100);
    }

    #[test]
    fn picks_nearest_supported_rate_across_discrete_ranges() {
        // Two discrete rates: 44.1 and 96. Request 48 → 44.1 is nearer (3.9k vs
        // 48k away).
        let ranges = [
            RateRange::new(44_100, 44_100),
            RateRange::new(96_000, 96_000),
        ];
        assert_eq!(select_capture_rate(48_000, &ranges, 0), 44_100);
        // Request 90k → 96 is nearer.
        assert_eq!(select_capture_rate(90_000, &ranges, 0), 96_000);
    }

    #[test]
    fn nearest_is_independent_of_range_order() {
        let a = [
            RateRange::new(96_000, 96_000),
            RateRange::new(44_100, 44_100),
        ];
        let b = [
            RateRange::new(44_100, 44_100),
            RateRange::new(96_000, 96_000),
        ];
        assert_eq!(
            select_capture_rate(48_000, &a, 0),
            select_capture_rate(48_000, &b, 0)
        );
    }

    #[test]
    fn ties_resolve_to_the_lower_rate_deterministically() {
        // Request exactly midway between 44.1 and 48 (distance 1950 each) →
        // the lower rate wins.
        let mid = (44_100 + 48_000) / 2; // 46_050, equidistant after integer div
        let ranges = [
            RateRange::new(44_100, 44_100),
            RateRange::new(48_000, 48_000),
        ];
        assert_eq!(select_capture_rate(mid, &ranges, 0), 44_100);
    }

    #[test]
    fn falls_back_to_default_when_no_ranges_advertised() {
        // Some backends return an empty supported-configs iterator; we then
        // trust the device's reported default config rate.
        assert_eq!(select_capture_rate(48_000, &[], 44_100), 44_100);
    }

    #[test]
    fn overlapping_ranges_covering_request_use_it_verbatim() {
        let ranges = [
            RateRange::new(8_000, 48_000),
            RateRange::new(44_100, 192_000),
        ];
        assert_eq!(select_capture_rate(96_000, &ranges, 0), 96_000);
    }

    #[test]
    fn swapped_bounds_are_normalised() {
        let r = RateRange::new(96_000, 44_100);
        assert_eq!(r.min, 44_100);
        assert_eq!(r.max, 96_000);
    }
}
