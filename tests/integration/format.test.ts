import { describe, it, expect } from "vitest";

import {
  clamp,
  formatTimecode,
  parseTimecode,
  formatDuration,
} from "@/lib/format";

describe("clamp", () => {
  it("bounds a value into range", () => {
    expect(clamp(5, 0, 10)).toBe(5);
    expect(clamp(-1, 0, 10)).toBe(0);
    expect(clamp(11, 0, 10)).toBe(10);
  });
});

describe("formatTimecode", () => {
  it("formats milliseconds as HH:MM:SS.mmm, zero-padded", () => {
    expect(formatTimecode(0)).toBe("00:00:00.000");
    expect(formatTimecode(1234)).toBe("00:00:01.234");
    expect(formatTimecode(61_000)).toBe("00:01:01.000");
    expect(formatTimecode(3_661_007)).toBe("01:01:01.007");
  });

  it("clamps negatives to zero", () => {
    expect(formatTimecode(-500)).toBe("00:00:00.000");
  });
});

describe("parseTimecode", () => {
  it("round-trips a full timecode", () => {
    expect(parseTimecode("01:01:01.007")).toBe(3_661_007);
  });

  it("accepts loose forms", () => {
    expect(parseTimecode("90")).toBe(90_000);
    expect(parseTimecode("1:30")).toBe(90_000);
    expect(parseTimecode("2:03.5")).toBe(123_500);
  });

  it("rejects garbage and out-of-range fields", () => {
    expect(parseTimecode("")).toBeNull();
    expect(parseTimecode("nope")).toBeNull();
    expect(parseTimecode("1:99")).toBeNull();
  });
});

describe("formatDuration", () => {
  it("drops the hour field when zero", () => {
    expect(formatDuration(42_000 + 7 * 60_000)).toBe("7:42");
    expect(formatDuration(50_000)).toBe("0:50");
  });

  it("shows hours when present", () => {
    expect(formatDuration(3_723_000)).toBe("1:02:03");
  });
});
