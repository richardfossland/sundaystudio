/**
 * Level-meter math — pure, so the broadcast colour zones and dBFS→position
 * mapping are unit-tested independently of the React component.
 *
 * Conventions: levels are in dBFS (0 = full scale, negative = quieter). The
 * meter floor is −60 dBFS by default; anything at or below reads as empty.
 * Zone thresholds follow common podcast/broadcast practice: nominal up to
 * −12, hot from −12 to −3, clipping risk above −3.
 */

import { clamp } from "./format";

export type MeterZone = "green" | "yellow" | "red";

export const METER_FLOOR_DB = -60;
const YELLOW_AT = -12;
const RED_AT = -3;

/** Which broadcast colour zone a dBFS level falls into. */
export function meterZone(db: number): MeterZone {
  if (db >= RED_AT) return "red";
  if (db >= YELLOW_AT) return "yellow";
  return "green";
}

/**
 * Map a dBFS level to a 0..1 fill fraction across the meter, where `floor`
 * reads as 0 and 0 dBFS reads as 1. Linear in dB (what meters look like), and
 * clamped so out-of-range input can't overflow the bar.
 */
export function dbToFraction(
  db: number,
  floor: number = METER_FLOOR_DB,
): number {
  if (floor >= 0) return 0; // guard against a nonsensical floor
  const f = (db - floor) / (0 - floor);
  return clamp(f, 0, 1);
}

/** CSS variable for a zone's colour — keeps components off the raw tokens. */
export function zoneColorVar(zone: MeterZone): string {
  return {
    green: "var(--color-meter-green)",
    yellow: "var(--color-meter-yellow)",
    red: "var(--color-meter-red)",
  }[zone];
}
