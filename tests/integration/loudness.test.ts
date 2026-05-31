import { describe, expect, it } from "vitest";

import {
  REACHED_TOLERANCE_LU,
  describeVerdict,
  effectivePeakDbtp,
  evaluateLoudness,
  roundLu,
} from "@/lib/loudness";
import type { LoudnessMeasurement, LoudnessTarget } from "@/lib/bindings";

/** A fully-populated measurement; spread + override per test. */
function measure(over: Partial<LoudnessMeasurement> = {}): LoudnessMeasurement {
  return {
    integrated_lufs: -20,
    short_term_lufs: -20,
    momentary_lufs: -20,
    loudness_range_lu: 6,
    true_peak_dbtp: -6,
    sample_peak_dbfs: -6,
    ...over,
  };
}

const spotify: LoudnessTarget = {
  id: "spotify",
  label: "Spotify",
  integrated_lufs: -14,
  true_peak_ceiling_dbtp: -1,
  description: "Spotify loudness normalisation",
};

describe("effectivePeakDbtp", () => {
  it("prefers true peak over sample peak", () => {
    expect(
      effectivePeakDbtp(measure({ true_peak_dbtp: -2, sample_peak_dbfs: -5 })),
    ).toBe(-2);
  });

  it("falls back to sample peak when true peak is missing", () => {
    expect(
      effectivePeakDbtp(
        measure({ true_peak_dbtp: null, sample_peak_dbfs: -5 }),
      ),
    ).toBe(-5);
  });

  it("is null when neither peak is available", () => {
    expect(
      effectivePeakDbtp(
        measure({ true_peak_dbtp: null, sample_peak_dbfs: null }),
      ),
    ).toBeNull();
  });
});

describe("evaluateLoudness", () => {
  it("reports too-quiet and the boost needed, when peak allows it", () => {
    // -20 measured, target -14 → wants +6 dB; peak -6 vs ceiling -1 → 5 dB headroom.
    // 6 > 5 so it caps at 5 dB.
    const v = evaluateLoudness(measure(), spotify);
    expect(v.status).toBe("too-quiet");
    expect(v.desiredGainDb).toBeCloseTo(6, 6);
    expect(v.peakHeadroomDb).toBeCloseTo(5, 6);
    expect(v.appliedGainDb).toBeCloseTo(5, 6);
    expect(v.gainCappedByPeak).toBe(true);
    expect(v.reachesTarget).toBe(false); // capped short of target
  });

  it("applies the full boost and reaches target when peak headroom is ample", () => {
    // Quiet program with a low peak: -20 LUFS, peak -20 → 19 dB headroom.
    const v = evaluateLoudness(
      measure({ true_peak_dbtp: -20, sample_peak_dbfs: -20 }),
      spotify,
    );
    expect(v.status).toBe("too-quiet");
    expect(v.desiredGainDb).toBeCloseTo(6, 6);
    expect(v.appliedGainDb).toBeCloseTo(6, 6);
    expect(v.gainCappedByPeak).toBe(false);
    expect(v.reachesTarget).toBe(true);
  });

  it("attenuates a too-loud program and never caps on peak", () => {
    // -8 LUFS, target -14 → wants -6 dB. Even a hot peak doesn't cap attenuation.
    const v = evaluateLoudness(
      measure({ integrated_lufs: -8, true_peak_dbtp: 0, sample_peak_dbfs: 0 }),
      spotify,
    );
    expect(v.status).toBe("too-loud");
    expect(v.desiredGainDb).toBeCloseTo(-6, 6);
    expect(v.appliedGainDb).toBeCloseTo(-6, 6);
    expect(v.gainCappedByPeak).toBe(false);
    expect(v.reachesTarget).toBe(true);
  });

  it("calls a program within tolerance on-target", () => {
    const v = evaluateLoudness(measure({ integrated_lufs: -14.3 }), spotify);
    expect(v.status).toBe("on-target");
    expect(Math.abs(v.desiredGainDb ?? 99)).toBeLessThanOrEqual(
      REACHED_TOLERANCE_LU,
    );
    expect(v.reachesTarget).toBe(true);
  });

  it("reports unmeasured when there is no integrated reading", () => {
    const v = evaluateLoudness(measure({ integrated_lufs: null }), spotify);
    expect(v.status).toBe("unmeasured");
    expect(v.desiredGainDb).toBeNull();
    expect(v.appliedGainDb).toBeNull();
    expect(v.reachesTarget).toBe(false);
    // Headroom still computable from the peak alone.
    expect(v.peakHeadroomDb).toBeCloseTo(5, 6);
  });

  it("still gives a boost when no peak reading is available (nothing to cap on)", () => {
    const v = evaluateLoudness(
      measure({ true_peak_dbtp: null, sample_peak_dbfs: null }),
      spotify,
    );
    expect(v.peakHeadroomDb).toBeNull();
    expect(v.appliedGainDb).toBeCloseTo(6, 6);
    expect(v.gainCappedByPeak).toBe(false);
  });

  it("does not mutate its inputs", () => {
    const m = measure();
    const t = { ...spotify };
    const snapshot = JSON.stringify({ m, t });
    evaluateLoudness(m, t);
    expect(JSON.stringify({ m, t })).toBe(snapshot);
  });
});

describe("roundLu", () => {
  it("rounds to one decimal and normalises -0 to 0", () => {
    expect(roundLu(3.44)).toBe(3.4);
    expect(roundLu(3.45)).toBeCloseTo(3.5, 6);
    expect(roundLu(-0.04)).toBe(0);
    expect(Object.is(roundLu(-0.04), -0)).toBe(false);
  });
});

describe("describeVerdict", () => {
  it("describes an on-target program", () => {
    expect(
      describeVerdict(
        evaluateLoudness(measure({ integrated_lufs: -14 }), spotify),
      ),
    ).toBe("On target (-14 LUFS).");
  });

  it("describes a too-quiet program with a clean boost", () => {
    const v = evaluateLoudness(
      measure({ true_peak_dbtp: -20, sample_peak_dbfs: -20 }),
      spotify,
    );
    expect(describeVerdict(v)).toBe(
      "6 LU too quiet — boost +6 dB to reach -14 LUFS.",
    );
  });

  it("describes a peak-capped boost and how far short it lands", () => {
    const v = evaluateLoudness(measure(), spotify); // caps 6→5, 1 LU short
    expect(describeVerdict(v)).toBe(
      "Capped at +5 dB by the true-peak ceiling; still 1 LU short of -14 LUFS.",
    );
  });

  it("describes a too-loud program", () => {
    const v = evaluateLoudness(measure({ integrated_lufs: -8 }), spotify);
    expect(describeVerdict(v)).toBe(
      "6 LU too loud — trim -6 dB to reach -14 LUFS.",
    );
  });

  it("describes the unmeasured case", () => {
    const v = evaluateLoudness(measure({ integrated_lufs: null }), spotify);
    expect(describeVerdict(v)).toBe("No loudness reading yet.");
  });
});
