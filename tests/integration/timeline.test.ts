import { describe, expect, it } from "vitest";

import {
  clampZoom,
  DEFAULT_PX_PER_SEC,
  formatTimecode,
  MAX_PX_PER_SEC,
  MIN_PX_PER_SEC,
  msToPx,
  pxToMs,
  rulerTicks,
  snap,
  tickIntervalSec,
} from "@/lib/timeline";

describe("timeline math", () => {
  it("maps ms↔px round-trip at a given zoom", () => {
    const z = 80;
    expect(msToPx(1000, z)).toBe(80);
    expect(pxToMs(80, z)).toBe(1000);
    expect(pxToMs(msToPx(3456, z), z)).toBeCloseTo(3456, 6);
  });

  it("guards px↔ms against zero/negative zoom", () => {
    expect(pxToMs(100, 0)).toBe(0);
  });

  it("clamps zoom to bounds and tolerates junk", () => {
    expect(clampZoom(1)).toBe(MIN_PX_PER_SEC);
    expect(clampZoom(99999)).toBe(MAX_PX_PER_SEC);
    expect(clampZoom(DEFAULT_PX_PER_SEC)).toBe(DEFAULT_PX_PER_SEC);
    expect(clampZoom(Number.NaN)).toBe(DEFAULT_PX_PER_SEC);
  });

  it("picks coarser ruler steps as you zoom out", () => {
    // Zoomed in: small step. Zoomed out: large step.
    const tight = tickIntervalSec(MAX_PX_PER_SEC);
    const wide = tickIntervalSec(MIN_PX_PER_SEC);
    expect(tight).toBeLessThan(wide);
    // At default zoom, labels stay reasonably spaced (step is a nice value).
    expect([1, 2, 5].includes(tickIntervalSec(DEFAULT_PX_PER_SEC))).toBe(true);
  });

  it("generates ruler ticks covering the full duration", () => {
    const ticks = rulerTicks(10_000, DEFAULT_PX_PER_SEC);
    expect(ticks[0].ms).toBe(0);
    expect(ticks.at(-1)!.ms).toBeGreaterThanOrEqual(10_000);
    // Steps are uniform.
    const step = ticks[1].ms - ticks[0].ms;
    expect(ticks[2].ms - ticks[1].ms).toBe(step);
  });

  it("formats timecode as m:ss and h:mm:ss", () => {
    expect(formatTimecode(0)).toBe("0:00");
    expect(formatTimecode(5_000)).toBe("0:05");
    expect(formatTimecode(95_000)).toBe("1:35");
    expect(formatTimecode(3_725_000)).toBe("1:02:05");
    expect(formatTimecode(-50)).toBe("0:00");
  });

  it("snaps to the nearest target within tolerance only", () => {
    const targets = [0, 1000, 2500];
    expect(snap(1040, targets, 50)).toBe(1000); // within tolerance
    expect(snap(1200, targets, 50)).toBe(1200); // nothing close enough
    expect(snap(2510, targets, 100)).toBe(2500);
  });
});
