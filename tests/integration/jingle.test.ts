/**
 * Jingle pipeline tests — pure offline logic, no hardware, no network.
 *
 * Covers:
 *   - validateJingleSpec: every constraint, boundary and happy-path
 *   - jingle_render_plan: stem list, ffmpeg args shape, naming, voiceover path
 *   - helpers: dbToLinear, sanitizeStemName
 */

import { describe, expect, it } from "vitest";

import {
  dbToLinear,
  jingle_render_plan,
  MAX_BPM,
  MAX_INSTRUMENTS,
  MIN_BPM,
  sanitizeStemName,
  VALID_DURATIONS,
  VALID_MOODS,
  validateJingleSpec,
  type JingleSpec,
} from "@/lib/jingle";

// ── Helpers ───────────────────────────────────────────────────────────────────

function spec(over: Partial<JingleSpec> = {}): JingleSpec {
  return {
    title: "Sunday Opener",
    duration_sec: 30,
    mood: "professional",
    tempo_bpm: 120,
    instruments: ["piano", "strings"],
    ...over,
  };
}

// ── validateJingleSpec ────────────────────────────────────────────────────────

describe("validateJingleSpec", () => {
  it("accepts a fully valid spec", () => {
    expect(validateJingleSpec(spec()).ok).toBe(true);
  });

  it("rejects an empty title", () => {
    const r = validateJingleSpec(spec({ title: "" }));
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.errors.some((e) => e.field === "title")).toBe(true);
  });

  it("rejects a whitespace-only title", () => {
    const r = validateJingleSpec(spec({ title: "   " }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "title_required")).toBe(true);
  });

  it("accepts all three valid durations", () => {
    for (const d of VALID_DURATIONS) {
      expect(validateJingleSpec(spec({ duration_sec: d })).ok).toBe(true);
    }
  });

  it("rejects an invalid duration (15)", () => {
    // Cast to bypass TS — at runtime the value may come from outside
    const r = validateJingleSpec(spec({ duration_sec: 15 as 20 | 30 | 60 }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "invalid_duration")).toBe(true);
  });

  it("accepts all four valid moods", () => {
    for (const m of VALID_MOODS) {
      expect(validateJingleSpec(spec({ mood: m })).ok).toBe(true);
    }
  });

  it("rejects an invalid mood", () => {
    const r = validateJingleSpec(
      spec({
        mood: "jazzy" as "energetic" | "calm" | "worshipful" | "professional",
      }),
    );
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "invalid_mood")).toBe(true);
  });

  it("accepts BPM on the inclusive minimum boundary", () => {
    expect(validateJingleSpec(spec({ tempo_bpm: MIN_BPM })).ok).toBe(true);
  });

  it("accepts BPM on the inclusive maximum boundary", () => {
    expect(validateJingleSpec(spec({ tempo_bpm: MAX_BPM })).ok).toBe(true);
  });

  it("rejects BPM below the minimum (59)", () => {
    const r = validateJingleSpec(spec({ tempo_bpm: 59 }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "bpm_out_of_range")).toBe(true);
  });

  it("rejects BPM above the maximum (201)", () => {
    const r = validateJingleSpec(spec({ tempo_bpm: 201 }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "bpm_out_of_range")).toBe(true);
  });

  it("rejects a non-integer BPM (120.5)", () => {
    const r = validateJingleSpec(spec({ tempo_bpm: 120.5 }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "bpm_not_integer")).toBe(true);
  });

  it("rejects a NaN BPM", () => {
    const r = validateJingleSpec(spec({ tempo_bpm: NaN }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "bpm_not_finite")).toBe(true);
  });

  it("rejects an empty instruments list", () => {
    const r = validateJingleSpec(spec({ instruments: [] }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "instruments_required")).toBe(
        true,
      );
  });

  it(`rejects more than ${MAX_INSTRUMENTS} instruments`, () => {
    const tooMany = Array.from(
      { length: MAX_INSTRUMENTS + 1 },
      (_, i) => `instr${i}`,
    );
    const r = validateJingleSpec(spec({ instruments: tooMany }));
    expect(r.ok).toBe(false);
    if (!r.ok)
      expect(r.errors.some((e) => e.code === "too_many_instruments")).toBe(
        true,
      );
  });

  it(`accepts exactly ${MAX_INSTRUMENTS} instruments`, () => {
    const exactly = Array.from({ length: MAX_INSTRUMENTS }, (_, i) => `i${i}`);
    expect(validateJingleSpec(spec({ instruments: exactly })).ok).toBe(true);
  });

  it("collects multiple errors in one pass (title + bpm)", () => {
    const r = validateJingleSpec(spec({ title: "", tempo_bpm: 300 }));
    expect(r.ok).toBe(false);
    if (!r.ok) {
      expect(r.errors.length).toBeGreaterThanOrEqual(2);
      expect(r.errors.some((e) => e.field === "title")).toBe(true);
      expect(r.errors.some((e) => e.field === "tempo_bpm")).toBe(true);
    }
  });

  it("accepts an optional voiceover_text (with content)", () => {
    expect(validateJingleSpec(spec({ voiceover_text: "Welcome!" })).ok).toBe(
      true,
    );
  });

  it("accepts a missing voiceover_text (undefined)", () => {
    const s = spec();
    delete s.voiceover_text;
    expect(validateJingleSpec(s).ok).toBe(true);
  });
});

// ── jingle_render_plan ────────────────────────────────────────────────────────

describe("jingle_render_plan", () => {
  it("returns the spec unchanged inside the plan", () => {
    const s = spec();
    const plan = jingle_render_plan(s, "out/test.wav");
    expect(plan.spec).toBe(s);
  });

  it("creates one stem per instrument", () => {
    const s = spec({ instruments: ["piano", "drums", "bass"] });
    const plan = jingle_render_plan(s, "out.wav");
    expect(
      plan.stems.filter((st) => !st.path.includes("voiceover")),
    ).toHaveLength(3);
  });

  it("maps instrument name to sanitized stem path", () => {
    const s = spec({ instruments: ["Grand Piano"] });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems[0].path).toBe("stems/grand_piano.wav");
  });

  it("adds a voiceover stem when voiceover_text is present", () => {
    const s = spec({ voiceover_text: "Good morning!" });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems.some((st) => st.path === "stems/voiceover.wav")).toBe(
      true,
    );
  });

  it("does NOT add a voiceover stem when voiceover_text is empty", () => {
    const s = spec({ voiceover_text: "" });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems.every((st) => st.path !== "stems/voiceover.wav")).toBe(
      true,
    );
  });

  it("does NOT add a voiceover stem when voiceover_text is whitespace", () => {
    const s = spec({ voiceover_text: "   " });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems.every((st) => st.path !== "stems/voiceover.wav")).toBe(
      true,
    );
  });

  it("uses the provided output path override", () => {
    const plan = jingle_render_plan(spec(), "custom/path.wav");
    expect(plan.output_path).toBe("custom/path.wav");
  });

  it("generates a default output path from title + duration when none given", () => {
    const plan = jingle_render_plan(
      spec({ title: "My Jingle", duration_sec: 20 }),
    );
    expect(plan.output_path).toContain("my_jingle");
    expect(plan.output_path).toContain("20s");
    expect(plan.output_path).toMatch(/\.wav$/);
  });

  it("ffmpeg_args starts with -y", () => {
    const plan = jingle_render_plan(spec(), "out.wav");
    expect(plan.ffmpeg_args[0]).toBe("-y");
  });

  it("ffmpeg_args contains one -i per stem", () => {
    const s = spec({ instruments: ["piano", "strings"] });
    const plan = jingle_render_plan(s, "out.wav");
    const inputCount = plan.ffmpeg_args.filter((a) => a === "-i").length;
    expect(inputCount).toBe(2); // two stems, no voiceover
  });

  it("ffmpeg_args ends with the output path", () => {
    const plan = jingle_render_plan(spec(), "final/output.wav");
    expect(plan.ffmpeg_args.at(-1)).toBe("final/output.wav");
  });

  it("ffmpeg_args includes atrim with the correct duration", () => {
    const plan = jingle_render_plan(spec({ duration_sec: 60 }), "out.wav");
    const filterArg = plan.ffmpeg_args.find((a) => a.includes("atrim=0:60"));
    expect(filterArg).toBeDefined();
  });

  it("ffmpeg_args includes 24-bit PCM codec", () => {
    const plan = jingle_render_plan(spec(), "out.wav");
    const codecIdx = plan.ffmpeg_args.indexOf("pcm_s24le");
    expect(codecIdx).toBeGreaterThan(-1);
    expect(plan.ffmpeg_args[codecIdx - 1]).toBe("-c:a");
  });

  it("ffmpeg_args contains 48 kHz sample rate", () => {
    const plan = jingle_render_plan(spec(), "out.wav");
    const arIdx = plan.ffmpeg_args.indexOf("-ar");
    expect(arIdx).toBeGreaterThan(-1);
    expect(plan.ffmpeg_args[arIdx + 1]).toBe("48000");
  });

  it("description includes title, duration, mood and tempo", () => {
    const s = spec({
      title: "Easter Special",
      duration_sec: 60,
      mood: "worshipful",
      tempo_bpm: 80,
    });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.description).toContain("Easter Special");
    expect(plan.description).toContain("60s");
    expect(plan.description).toContain("worshipful");
    expect(plan.description).toContain("80 BPM");
  });

  it("adds voiceover duck mix to filter_complex when voiceover present", () => {
    const s = spec({ voiceover_text: "Good morning!" });
    const plan = jingle_render_plan(s, "out.wav");
    const filterArg = plan.ffmpeg_args.find(
      (a) => typeof a === "string" && a.includes("volume=0.5"),
    );
    expect(filterArg).toBeDefined();
  });

  it("drums get a gain boost in energetic mode", () => {
    const s = spec({ instruments: ["drums"], mood: "energetic" });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems[0].gain_db).toBeGreaterThan(0);
  });

  it("drums get a gain cut in calm mode", () => {
    const s = spec({ instruments: ["drums"], mood: "calm" });
    const plan = jingle_render_plan(s, "out.wav");
    expect(plan.stems[0].gain_db).toBeLessThan(0);
  });
});

// ── dbToLinear helper ─────────────────────────────────────────────────────────

describe("dbToLinear", () => {
  it("0 dB maps to 1.0 (unity gain)", () => {
    expect(dbToLinear(0)).toBeCloseTo(1.0, 5);
  });

  it("-6 dB is approximately 0.5 (half amplitude)", () => {
    expect(dbToLinear(-6)).toBeCloseTo(0.5012, 3);
  });

  it("-20 dB maps to 0.1", () => {
    expect(dbToLinear(-20)).toBeCloseTo(0.1, 5);
  });

  it("+6 dB is approximately 2.0 (double amplitude)", () => {
    expect(dbToLinear(6)).toBeCloseTo(1.995, 2);
  });
});

// ── sanitizeStemName helper ───────────────────────────────────────────────────

describe("sanitizeStemName", () => {
  it("lowercases and trims", () => {
    expect(sanitizeStemName("  Piano  ")).toBe("piano");
  });

  it("replaces spaces with underscores", () => {
    expect(sanitizeStemName("Grand Piano")).toBe("grand_piano");
  });

  it("replaces non-safe chars with underscores and collapses runs", () => {
    // '!' is not in [a-z0-9_-] → replaced with '_', then runs collapsed
    expect(sanitizeStemName("bass!!line")).toBe("bass_line");
  });

  it("handles an empty string gracefully", () => {
    expect(sanitizeStemName("")).toBe("untitled");
  });

  it("handles a string with only special chars", () => {
    expect(sanitizeStemName("!!!")).toBe("untitled");
  });

  it("keeps hyphens and underscores intact", () => {
    expect(sanitizeStemName("hi_hat-ride")).toBe("hi_hat-ride");
  });
});
