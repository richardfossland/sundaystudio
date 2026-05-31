/**
 * Pure loudness-target reasoning (Phase 4.2) — the math that turns a measured
 * `LoudnessMeasurement` plus a `LoudnessTarget` into a human-facing verdict the
 * Export / Diagnostics surface can show *before* committing to a render:
 *   - how much normalisation gain we'd apply to hit the integrated-loudness goal
 *   - whether that gain is reachable clip-safe (true-peak ceiling), and if not,
 *     how much it gets capped
 *   - the headroom against the ceiling, and a coarse compliance status
 *
 * This mirrors the semantics the backend `NormalizationReport` reports after an
 * actual pass (same 0.5 LU "reached" tolerance), so a UI preview computed here
 * agrees with what the render produces. Everything is pure and unit-tested; no
 * IPC, no DOM. The backend remains the source of truth for the real render — this
 * is the cheap, immediate estimate the picker uses to guide the user.
 */
import type { LoudnessMeasurement, LoudnessTarget } from "./bindings";

/** Within this many LU of target counts as "on target" (matches the backend). */
export const REACHED_TOLERANCE_LU = 0.5;

/** How a measured program sits relative to a platform's loudness target. */
export type LoudnessStatus =
  | "on-target" // integrated loudness already within tolerance
  | "too-quiet" // below target — we can raise it (peak permitting)
  | "too-loud" // above target — we'd attenuate
  | "unmeasured"; // no integrated reading yet (silence / unfilled window)

/**
 * A preview verdict for normalising `measurement` to `target`. All gains are in
 * dB (positive = boost), all loudness in LUFS/LU, peaks in dBTP.
 */
export interface LoudnessVerdict {
  status: LoudnessStatus;
  /** The target we evaluated against (echoed for the UI). */
  targetLufs: number;
  /** Measured integrated loudness, or null when unmeasured. */
  measuredLufs: number | null;
  /**
   * Gain we'd *like* to apply to hit the target exactly (target − measured).
   * Null when unmeasured.
   */
  desiredGainDb: number | null;
  /**
   * Gain we can actually apply without the true peak crossing the ceiling.
   * Equals `desiredGainDb` unless a boost would clip, in which case it's capped
   * to the available peak headroom. Null when unmeasured.
   */
  appliedGainDb: number | null;
  /** True when a wanted boost was held back by the true-peak ceiling. */
  gainCappedByPeak: boolean;
  /**
   * Headroom between the current true peak and the ceiling, in dB (positive =
   * room to spare). Null when there's no true-peak reading. Attenuation always
   * increases headroom, so a negative `appliedGainDb` never caps.
   */
  peakHeadroomDb: number | null;
  /** True when the achieved loudness lands within tolerance of the target. */
  reachesTarget: boolean;
}

/**
 * Pick the most reliable peak reading: prefer the true (inter-sample) peak —
 * the metric platforms actually police — falling back to the sample peak when
 * true peak wasn't computed. Null when neither is available.
 */
export function effectivePeakDbtp(m: LoudnessMeasurement): number | null {
  if (m.true_peak_dbtp != null) return m.true_peak_dbtp;
  if (m.sample_peak_dbfs != null) return m.sample_peak_dbfs;
  return null;
}

/**
 * Evaluate normalising `measurement` toward `target`. Pure: given the same
 * inputs it always yields the same verdict, and it never mutates its arguments.
 */
export function evaluateLoudness(
  measurement: LoudnessMeasurement,
  target: LoudnessTarget,
): LoudnessVerdict {
  const targetLufs = target.integrated_lufs;
  const measuredLufs = measurement.integrated_lufs;
  const peak = effectivePeakDbtp(measurement);
  const ceiling = target.true_peak_ceiling_dbtp;

  // Headroom is meaningful regardless of whether we have a loudness reading.
  const peakHeadroomDb = peak == null ? null : ceiling - peak;

  if (measuredLufs == null) {
    return {
      status: "unmeasured",
      targetLufs,
      measuredLufs: null,
      desiredGainDb: null,
      appliedGainDb: null,
      gainCappedByPeak: false,
      peakHeadroomDb,
      reachesTarget: false,
    };
  }

  const desiredGainDb = targetLufs - measuredLufs;

  // A boost can only go as far as the peak headroom allows. Attenuation (or no
  // peak reading to police) is always safe.
  let appliedGainDb = desiredGainDb;
  let gainCappedByPeak = false;
  if (
    desiredGainDb > 0 &&
    peakHeadroomDb != null &&
    desiredGainDb > peakHeadroomDb
  ) {
    appliedGainDb = Math.max(0, peakHeadroomDb);
    gainCappedByPeak = true;
  }

  const achievedLufs = measuredLufs + appliedGainDb;
  const reachesTarget =
    Math.abs(achievedLufs - targetLufs) <= REACHED_TOLERANCE_LU;

  let status: LoudnessStatus;
  if (Math.abs(desiredGainDb) <= REACHED_TOLERANCE_LU) status = "on-target";
  else if (desiredGainDb > 0) status = "too-quiet";
  else status = "too-loud";

  return {
    status,
    targetLufs,
    measuredLufs,
    desiredGainDb,
    appliedGainDb,
    gainCappedByPeak,
    peakHeadroomDb,
    reachesTarget,
  };
}

/** Round a dB/LU figure to one decimal for display (avoids `-0`). */
export function roundLu(value: number): number {
  const r = Math.round(value * 10) / 10;
  return r === 0 ? 0 : r;
}

/**
 * A short, plain-language line summarising a verdict, e.g.
 *   "On target (−16 LUFS)."
 *   "3.4 LU too quiet — boost +3.4 dB to reach −16 LUFS."
 *   "Capped at +1.2 dB by the −1.0 dBTP true-peak ceiling; still 2.2 LU short."
 * Pure and deterministic — handy for the picker subtitle and for asserting on
 * the formatting in tests.
 */
export function describeVerdict(v: LoudnessVerdict): string {
  const tgt = `${roundLu(v.targetLufs)} LUFS`;
  if (v.status === "unmeasured") return "No loudness reading yet.";
  if (v.status === "on-target") return `On target (${tgt}).`;

  const desired = v.desiredGainDb ?? 0;
  const gap = Math.abs(roundLu(desired));

  if (v.status === "too-loud") {
    return `${gap} LU too loud — trim ${roundLu(desired)} dB to reach ${tgt}.`;
  }

  // too-quiet
  if (v.gainCappedByPeak) {
    const applied = roundLu(v.appliedGainDb ?? 0);
    const short = roundLu(desired - (v.appliedGainDb ?? 0));
    return `Capped at +${applied} dB by the true-peak ceiling; still ${short} LU short of ${tgt}.`;
  }
  return `${gap} LU too quiet — boost +${gap} dB to reach ${tgt}.`;
}
