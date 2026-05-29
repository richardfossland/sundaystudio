/**
 * Pure formatting helpers for the audio domain. Kept dependency-free and
 * side-effect-free so they are trivially unit-testable and reusable from any
 * component (timecodes, durations, file sizes).
 */

/** Clamp a number into [min, max]. */
export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/**
 * Format a millisecond position as a broadcast timecode `HH:MM:SS.mmm`.
 * Negative input clamps to zero. Always zero-padded so the monospaced display
 * never jitters in width.
 */
export function formatTimecode(ms: number): string {
  const total = Math.max(0, Math.round(ms));
  const millis = total % 1000;
  const totalSeconds = Math.floor(total / 1000);
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);
  const p2 = (n: number) => n.toString().padStart(2, "0");
  const p3 = (n: number) => n.toString().padStart(3, "0");
  return `${p2(hours)}:${p2(minutes)}:${p2(seconds)}.${p3(millis)}`;
}

/**
 * Parse a timecode back to milliseconds. Accepts, from loose to full:
 *   - `SS[.mmm]`        bare seconds, any magnitude (`90` → 90 s)
 *   - `MM:SS[.mmm]`     minutes:seconds (`1:30` → 90 s)
 *   - `HH:MM:SS[.mmm]`  full
 * When a colon is present, the seconds field (and minutes, when hours are
 * present) must be 0–59. Returns null on anything it can't parse — callers
 * keep the previous value on null, so the transport can't be driven to a
 * nonsense position.
 */
export function parseTimecode(text: string): number | null {
  const trimmed = text.trim();
  if (trimmed === "") return null;

  const parts = trimmed.split(":");
  if (parts.length > 3) return null;

  // The final segment carries seconds + an optional .mmm fraction.
  const secMatch = parts[parts.length - 1].match(/^(\d+)(?:\.(\d{1,3}))?$/);
  if (!secMatch) return null;
  const seconds = Number(secMatch[1]);
  const millis = secMatch[2] ? Number(secMatch[2].padEnd(3, "0")) : 0;

  const higher = parts.slice(0, -1).map(Number);
  if (higher.some((n) => Number.isNaN(n) || n < 0)) return null;

  let minutes = 0;
  let hours = 0;
  if (parts.length === 2) {
    if (seconds > 59) return null;
    minutes = higher[0];
  } else if (parts.length === 3) {
    if (seconds > 59 || higher[1] > 59) return null;
    hours = higher[0];
    minutes = higher[1];
  }

  return ((hours * 60 + minutes) * 60 + seconds) * 1000 + millis;
}

/** Format a short, human duration like `42:50` or `1:02:03` (no millis). */
export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.round(ms / 1000));
  const seconds = totalSeconds % 60;
  const minutes = Math.floor(totalSeconds / 60) % 60;
  const hours = Math.floor(totalSeconds / 3600);
  const p2 = (n: number) => n.toString().padStart(2, "0");
  return hours > 0
    ? `${hours}:${p2(minutes)}:${p2(seconds)}`
    : `${minutes}:${p2(seconds)}`;
}
