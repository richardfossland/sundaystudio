//! Silence detection (Phase 3.3) — find the quiet gaps in a recording so the
//! editor can trim them out, the single biggest time-saver in podcast cleanup.
//!
//! The detector is deliberately conservative: it scans short windows, marks a
//! window silent when its peak sits below a dBFS threshold, and only reports a
//! gap once a run of silent windows lasts at least `min_silence_ms`. The default
//! threshold (−50 dBFS) and minimum length (500 ms) err toward *missing* some
//! silence rather than ever cutting into speech — a false cut is far worse than
//! a kept breath. Breath-level detection (a bundled ONNX model) is a later step;
//! this pure, level-based pass needs no model and is fully testable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A silent span within a take's audio, in milliseconds (take-relative).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/SilenceSpan.ts")]
pub struct SilenceSpan {
    pub start_ms: f64,
    pub end_ms: f64,
}

/// Analysis window, in seconds (10 ms gives ~480 samples at 48 kHz).
const WINDOW_SECS: f64 = 0.010;

/// Detect silences: windows whose peak amplitude is below `threshold_db` (dBFS),
/// coalesced into spans of at least `min_silence_ms`. Pure over the mono samples.
pub fn detect_silences(
    samples: &[f32],
    rate: u32,
    threshold_db: f32,
    min_silence_ms: f64,
) -> Vec<SilenceSpan> {
    if samples.is_empty() || rate == 0 {
        return Vec::new();
    }
    let win = ((rate as f64 * WINDOW_SECS) as usize).max(1);
    let thresh = 10f32.powf(threshold_db / 20.0); // dBFS → linear amplitude

    let mut spans = Vec::new();
    let mut run_start: Option<usize> = None; // sample index where the gap began
    let n = samples.len();
    let mut i = 0;
    while i < n {
        let end = (i + win).min(n);
        let peak = samples[i..end].iter().fold(0f32, |m, &s| m.max(s.abs()));
        if peak < thresh {
            run_start.get_or_insert(i);
        } else if let Some(start) = run_start.take() {
            push_span(&mut spans, start, i, rate, min_silence_ms);
        }
        i = end;
    }
    if let Some(start) = run_start.take() {
        push_span(&mut spans, start, n, rate, min_silence_ms);
    }
    spans
}

fn push_span(spans: &mut Vec<SilenceSpan>, start: usize, end: usize, rate: u32, min_ms: f64) {
    let start_ms = start as f64 / rate as f64 * 1000.0;
    let end_ms = end as f64 / rate as f64 * 1000.0;
    if end_ms - start_ms >= min_ms {
        spans.push(SilenceSpan { start_ms, end_ms });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn block(amp: f32, secs: f64) -> Vec<f32> {
        vec![amp; (SR as f64 * secs) as usize]
    }

    #[test]
    fn finds_a_silent_gap_between_loud_sections() {
        let mut s = block(0.5, 0.3); // loud
        s.extend(block(0.0, 0.8)); // silent ≥ 500ms
        s.extend(block(0.5, 0.3)); // loud
        let spans = detect_silences(&s, SR, -50.0, 500.0);
        assert_eq!(spans.len(), 1);
        let gap = spans[0];
        assert!((gap.start_ms - 300.0).abs() < 20.0, "start {}", gap.start_ms);
        assert!((gap.end_ms - 1100.0).abs() < 20.0, "end {}", gap.end_ms);
    }

    #[test]
    fn ignores_silence_shorter_than_the_minimum() {
        let mut s = block(0.5, 0.3);
        s.extend(block(0.0, 0.1)); // 100ms gap < 500ms minimum
        s.extend(block(0.5, 0.3));
        assert!(detect_silences(&s, SR, -50.0, 500.0).is_empty());
    }

    #[test]
    fn low_level_hum_below_threshold_counts_as_silence() {
        // −60 dBFS noise floor is below a −50 dBFS threshold → still "silent".
        let floor = 10f32.powf(-60.0 / 20.0);
        let s = block(floor, 1.0);
        let spans = detect_silences(&s, SR, -50.0, 500.0);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].end_ms - spans[0].start_ms >= 500.0);
    }

    #[test]
    fn fully_loud_signal_has_no_silence() {
        assert!(detect_silences(&block(0.4, 1.0), SR, -50.0, 500.0).is_empty());
    }

    #[test]
    fn empty_or_zero_rate_is_safe() {
        assert!(detect_silences(&[], SR, -50.0, 500.0).is_empty());
        assert!(detect_silences(&block(0.0, 1.0), 0, -50.0, 500.0).is_empty());
    }
}
