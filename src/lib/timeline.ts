/**
 * Pure timeline math for the Phase 3.1 editor: zoom, millisecond⇄pixel mapping,
 * ruler tick selection, snapping, and timecode formatting. No React, no DOM —
 * every function here is deterministic and unit-tested, so the canvas/UI layers
 * stay thin.
 *
 * Zoom is expressed as **pixels per second** of audio. The timeline ruler keeps
 * ticks at a comfortable spacing by stepping through "nice" intervals as you
 * zoom, the way every DAW does.
 */

/** Zoom bounds, in pixels per second. */
export const MIN_PX_PER_SEC = 4; // ~4 min visible in 1000px (overview)
export const MAX_PX_PER_SEC = 400; // ~2.5s visible in 1000px (sample-close)
export const DEFAULT_PX_PER_SEC = 60;

/** Multiplicative step for zoom in/out buttons and wheel. */
export const ZOOM_FACTOR = 1.5;

export function clampZoom(pxPerSec: number): number {
  if (!Number.isFinite(pxPerSec)) return DEFAULT_PX_PER_SEC;
  return Math.min(MAX_PX_PER_SEC, Math.max(MIN_PX_PER_SEC, pxPerSec));
}

export function msToPx(ms: number, pxPerSec: number): number {
  return (ms / 1000) * pxPerSec;
}

export function pxToMs(px: number, pxPerSec: number): number {
  if (pxPerSec <= 0) return 0;
  return (px / pxPerSec) * 1000;
}

/** "Nice" ruler steps in seconds — human-friendly multiples only. */
const NICE_SECONDS = [
  0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 1200, 1800, 3600,
];

/**
 * Pick a ruler step (seconds) so labels sit roughly `targetPx` apart at the
 * current zoom. Returns the smallest nice step whose pixel width clears the
 * target, falling back to the coarsest step when very zoomed out.
 */
export function tickIntervalSec(pxPerSec: number, targetPx = 90): number {
  const idealSec = targetPx / Math.max(pxPerSec, 0.0001);
  for (const s of NICE_SECONDS) {
    if (s >= idealSec) return s;
  }
  return NICE_SECONDS[NICE_SECONDS.length - 1];
}

export interface Tick {
  ms: number;
  label: string;
}

/**
 * Ruler ticks from 0 to `durationMs` (inclusive of the final step boundary that
 * covers the duration), spaced by the zoom-appropriate nice interval.
 */
export function rulerTicks(durationMs: number, pxPerSec: number): Tick[] {
  const stepMs = tickIntervalSec(pxPerSec) * 1000;
  if (stepMs <= 0) return [];
  const ticks: Tick[] = [];
  const end = Math.max(0, durationMs);
  for (let ms = 0; ms <= end + stepMs; ms += stepMs) {
    ticks.push({ ms, label: formatTimecode(ms) });
    if (ticks.length > 10_000) break; // guardrail against pathological input
  }
  return ticks;
}

function pad(n: number): string {
  return n.toString().padStart(2, "0");
}

/** Format milliseconds as `m:ss` (or `h:mm:ss` past an hour). */
export function formatTimecode(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}

/**
 * Snap `ms` to the nearest target within `toleranceMs`; returns `ms` unchanged
 * if nothing is close enough. Used when moving/trimming regions against region
 * edges, markers, and the playhead.
 */
export function snap(
  ms: number,
  targets: number[],
  toleranceMs: number,
): number {
  let best = ms;
  let bestDist = toleranceMs;
  for (const t of targets) {
    const d = Math.abs(t - ms);
    if (d <= bestDist) {
      bestDist = d;
      best = t;
    }
  }
  return best;
}
