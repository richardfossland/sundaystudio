import { describe, it, expect } from "vitest";

import { meterZone, dbToFraction, METER_FLOOR_DB } from "@/lib/meter";

describe("meterZone", () => {
  it("maps dBFS to broadcast zones", () => {
    expect(meterZone(-40)).toBe("green");
    expect(meterZone(-12)).toBe("yellow");
    expect(meterZone(-6)).toBe("yellow");
    expect(meterZone(-3)).toBe("red");
    expect(meterZone(0)).toBe("red");
  });
});

describe("dbToFraction", () => {
  it("maps the floor to 0 and full scale to 1", () => {
    expect(dbToFraction(METER_FLOOR_DB)).toBe(0);
    expect(dbToFraction(0)).toBe(1);
  });

  it("is linear in dB across the range", () => {
    // Halfway between -60 and 0 is -30 → 0.5
    expect(dbToFraction(-30)).toBeCloseTo(0.5, 5);
  });

  it("clamps out-of-range input", () => {
    expect(dbToFraction(12)).toBe(1);
    expect(dbToFraction(-120)).toBe(0);
  });
});
