//! Fade curves for clip edges and crossfades (Phase 7.1 refinement).
//!
//! The render path bakes a fade-in/out into each clip's edges; where two clips
//! overlap with matching fades, the overlap is a crossfade. Today those fades
//! are **linear** (gain ramps 0→1 in a straight line). For a hard cut against
//! silence that is fine, but for a *crossfade* — one clip fading out while the
//! next fades in — linear gain has a well-known flaw: at the midpoint both clips
//! sit at 0.5 gain, so for uncorrelated material (two different takes, a music
//! bed under a voiceover) the summed **power** dips to ≈ −3 dB. You hear a hole.
//!
//! [`FadeShape::EqualPower`] fixes this: each side follows a quarter-sine so that
//! `gain_a(t)² + gain_b(t)² == 1` across the whole crossfade — constant power, no
//! dip. This is the "equal-power is a later refinement" note in
//! `src/lib/editing.ts::crossfadeOps` made real on the render side.
//!
//! Everything here is pure and unit-tested — no audio device, no I/O. The
//! existing linear `render::render_region` is untouched; callers opt into a
//! shape via [`apply_fades`].

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The curve a fade ramp follows from silence (0.0) to unity (1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS, Default)]
#[ts(export, export_to = "../../src/lib/bindings/FadeShape.ts")]
#[serde(rename_all = "snake_case")]
pub enum FadeShape {
    /// Straight-line gain. Click-free for a single edge; dips ≈ −3 dB at the
    /// midpoint of a crossfade of uncorrelated material.
    #[default]
    Linear,
    /// Quarter-sine gain so a matched fade-out/fade-in pair sums to constant
    /// power (no midpoint dip). The right default for crossfades.
    EqualPower,
}

impl FadeShape {
    /// Gain in `[0.0, 1.0]` at normalised fade progress `t` (clamped to that
    /// range), where `t = 0.0` is the start of the fade-in (silence) and
    /// `t = 1.0` is fully open (unity).
    ///
    /// - `Linear`: `g = t`
    /// - `EqualPower`: `g = sin(t · π/2)` — so `g(0)=0`, `g(1)=1`, and a matched
    ///   fade-out (`g(1−t)`) gives `sin² + cos² = 1`.
    pub fn gain_in(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            FadeShape::Linear => t,
            FadeShape::EqualPower => (t * std::f32::consts::FRAC_PI_2).sin(),
        }
    }

    /// Gain at normalised progress `t` through a fade-*out*: the mirror of
    /// [`gain_in`](Self::gain_in), so `t = 0.0` is unity and `t = 1.0` is
    /// silence. `gain_out(t) == gain_in(1 − t)`.
    pub fn gain_out(self, t: f32) -> f32 {
        self.gain_in(1.0 - t.clamp(0.0, 1.0))
    }
}

/// Apply a fade-in of `fade_in` samples and a fade-out of `fade_out` samples to
/// `buf`, in place, using `shape`. Each ramp is clamped to the buffer length;
/// if the two ramps would overlap they simply multiply (a very short clip with
/// long fades still ends up quieter in the middle, never louder). Pure.
pub fn apply_fades(buf: &mut [f32], fade_in: usize, fade_out: usize, shape: FadeShape) {
    let n = buf.len();
    let fade_in = fade_in.min(n);
    for (i, x) in buf.iter_mut().take(fade_in).enumerate() {
        // i/fade_in spans [0, 1); the sample at the very start is silence.
        *x *= shape.gain_in(i as f32 / fade_in as f32);
    }
    let fade_out = fade_out.min(n);
    for k in 0..fade_out {
        // k counts back from the end: k = 0 is the last sample (silence).
        buf[n - 1 - k] *= shape.gain_in(k as f32 / fade_out as f32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-5;

    #[test]
    fn endpoints_are_silence_and_unity() {
        for shape in [FadeShape::Linear, FadeShape::EqualPower] {
            assert!(shape.gain_in(0.0).abs() < EPS, "{shape:?} in(0)");
            assert!((shape.gain_in(1.0) - 1.0).abs() < EPS, "{shape:?} in(1)");
            assert!((shape.gain_out(0.0) - 1.0).abs() < EPS, "{shape:?} out(0)");
            assert!(shape.gain_out(1.0).abs() < EPS, "{shape:?} out(1)");
        }
    }

    #[test]
    fn t_is_clamped_outside_unit_range() {
        assert_eq!(FadeShape::Linear.gain_in(-0.5), 0.0);
        assert_eq!(FadeShape::Linear.gain_in(2.0), 1.0);
        assert!(FadeShape::EqualPower.gain_in(-1.0).abs() < EPS);
        assert!((FadeShape::EqualPower.gain_in(9.0) - 1.0).abs() < EPS);
    }

    #[test]
    fn linear_is_a_straight_line() {
        assert!((FadeShape::Linear.gain_in(0.25) - 0.25).abs() < EPS);
        assert!((FadeShape::Linear.gain_in(0.5) - 0.5).abs() < EPS);
        assert!((FadeShape::Linear.gain_in(0.75) - 0.75).abs() < EPS);
    }

    #[test]
    fn linear_crossfade_dips_at_the_midpoint() {
        // The bug equal-power fixes: linear in+out both = 0.5 at the centre, so
        // summed power for uncorrelated material is 0.5 (≈ −3 dB), not 1.0.
        let g_in = FadeShape::Linear.gain_in(0.5);
        let g_out = FadeShape::Linear.gain_out(0.5);
        let power = g_in * g_in + g_out * g_out;
        assert!((power - 0.5).abs() < EPS, "linear midpoint power {power}");
    }

    #[test]
    fn equal_power_crossfade_is_constant_power() {
        // The whole point: gain_in(t)² + gain_out(t)² == 1 for every t, so a
        // matched fade-out/fade-in pair never dips or bumps in power.
        for i in 0..=20 {
            let t = i as f32 / 20.0;
            let g_in = FadeShape::EqualPower.gain_in(t);
            let g_out = FadeShape::EqualPower.gain_out(t);
            let power = g_in * g_in + g_out * g_out;
            assert!((power - 1.0).abs() < EPS, "t={t} power={power}");
        }
    }

    #[test]
    fn equal_power_midpoint_is_minus_3db() {
        // At the centre each side sits at sin(π/4) ≈ 0.707 (−3 dB amplitude),
        // which is exactly what makes the powers sum to unity.
        let g = FadeShape::EqualPower.gain_in(0.5);
        assert!((g - std::f32::consts::FRAC_1_SQRT_2).abs() < EPS, "mid {g}");
    }

    #[test]
    fn gain_in_is_monotonic_non_decreasing() {
        for shape in [FadeShape::Linear, FadeShape::EqualPower] {
            let mut prev = -1.0;
            for i in 0..=50 {
                let g = shape.gain_in(i as f32 / 50.0);
                assert!(
                    g >= prev - EPS,
                    "{shape:?} not monotonic at {i}: {g} < {prev}"
                );
                prev = g;
            }
        }
    }

    #[test]
    fn apply_fades_ramps_both_edges() {
        let mut buf = vec![1.0_f32; 100];
        apply_fades(&mut buf, 20, 20, FadeShape::EqualPower);
        // Edges faded toward silence, centre untouched.
        assert!(buf[0].abs() < EPS);
        assert!(buf[99].abs() < EPS);
        assert!((buf[50] - 1.0).abs() < EPS);
        // Equal-power: a quarter of the way in sits above the linear line.
        assert!(buf[5] > 5.0 / 20.0, "equal-power should lead linear early");
    }

    #[test]
    fn apply_fades_default_shape_matches_existing_linear_render() {
        // Default == Linear, so opting in without choosing keeps today's behaviour
        // (the gain at i/fade matches render_region's `i as f32 / fade_in`).
        let mut buf = vec![1.0_f32; 10];
        apply_fades(&mut buf, 4, 0, FadeShape::default());
        assert!(buf[0].abs() < EPS);
        assert!((buf[1] - 0.25).abs() < EPS);
        assert!((buf[2] - 0.5).abs() < EPS);
        assert!((buf[3] - 0.75).abs() < EPS);
        assert!((buf[4] - 1.0).abs() < EPS);
    }

    #[test]
    fn apply_fades_clamps_ramps_to_buffer_length() {
        // Fades longer than the buffer must not panic or read out of bounds.
        let mut buf = vec![1.0_f32; 4];
        apply_fades(&mut buf, 100, 100, FadeShape::EqualPower);
        assert_eq!(buf.len(), 4);
        assert!(buf[0].abs() < EPS); // start still silenced
    }

    #[test]
    fn apply_fades_on_empty_buffer_is_a_noop() {
        let mut buf: Vec<f32> = Vec::new();
        apply_fades(&mut buf, 10, 10, FadeShape::Linear);
        assert!(buf.is_empty());
    }
}
