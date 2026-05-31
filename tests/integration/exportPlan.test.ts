import { describe, expect, it } from "vitest";

import {
  ALLOWED_SAMPLE_RATES,
  buildFfmpegArgs,
  DEFAULT_SAMPLE_RATE,
  describePlan,
  ffmpegCodec,
  formatExtension,
  isLossless,
  MAX_BITRATE_KBPS,
  MIN_BITRATE_KBPS,
  planExport,
  type ExportPlan,
} from "@/lib/exportPlan";
import type { ExportPresetInfo } from "@/lib/bindings";

function preset(over: Partial<ExportPresetInfo> = {}): ExportPresetInfo {
  return {
    id: "spotify",
    label: "Spotify for Podcasters",
    format: "mp3",
    bitrate_kbps: 192,
    channels: 2,
    target_id: "spotify",
    description: "192 kbps MP3, stereo, -14 LUFS. Ready to upload.",
    requires_encoder: true,
    ...over,
  };
}

/** Narrow a plan result or fail the test (keeps assertions readable). */
function planOf(result: ReturnType<typeof planExport>): ExportPlan {
  if ("error" in result) {
    throw new Error(`expected a plan, got error ${result.error.code}`);
  }
  return result.plan;
}

describe("export plan: format helpers", () => {
  it("classifies lossless vs lossy", () => {
    expect(isLossless("wav")).toBe(true);
    expect(isLossless("flac")).toBe(true);
    expect(isLossless("mp3")).toBe(false);
    expect(isLossless("aac")).toBe(false);
  });

  it("maps each format to its ffmpeg codec", () => {
    expect(ffmpegCodec("wav")).toBe("pcm_s24le");
    expect(ffmpegCodec("mp3")).toBe("libmp3lame");
    expect(ffmpegCodec("aac")).toBe("aac");
    expect(ffmpegCodec("flac")).toBe("flac");
  });

  it("mirrors the Rust extension mapping (aac -> m4a)", () => {
    expect(formatExtension("wav")).toBe("wav");
    expect(formatExtension("mp3")).toBe("mp3");
    expect(formatExtension("aac")).toBe("m4a");
    expect(formatExtension("flac")).toBe("flac");
  });
});

describe("export plan: validation", () => {
  it("resolves a valid lossy preset at the default sample rate", () => {
    const plan = planOf(planExport(preset()));
    expect(plan.format).toBe("mp3");
    expect(plan.channels).toBe(2);
    expect(plan.sampleRate).toBe(DEFAULT_SAMPLE_RATE);
    expect(plan.bitrateKbps).toBe(192);
    expect(plan.codec).toBe("libmp3lame");
    expect(plan.requiresEncoder).toBe(true);
    expect(plan.extension).toBe("mp3");
  });

  it("resolves a WAV preset as native with no bitrate", () => {
    const plan = planOf(
      planExport(preset({ format: "wav", bitrate_kbps: null })),
    );
    expect(plan.requiresEncoder).toBe(false);
    expect(plan.bitrateKbps).toBeNull();
    expect(plan.codec).toBe("pcm_s24le");
    expect(plan.extension).toBe("wav");
  });

  it("resolves a FLAC preset as a lossless encoder pass", () => {
    const plan = planOf(
      planExport(preset({ format: "flac", bitrate_kbps: null })),
    );
    expect(plan.requiresEncoder).toBe(true); // FLAC still goes via ffmpeg
    expect(plan.bitrateKbps).toBeNull();
  });

  it("accepts every allowed sample rate", () => {
    for (const rate of ALLOWED_SAMPLE_RATES) {
      const plan = planOf(planExport(preset(), rate));
      expect(plan.sampleRate).toBe(rate);
    }
  });

  it("rejects an unsupported sample rate", () => {
    const r = planExport(preset(), 96000);
    expect("error" in r && r.error.code).toBe("bad_sample_rate");
  });

  it("rejects a channel count that isn't mono or stereo", () => {
    const r = planExport(preset({ channels: 6 }));
    expect("error" in r && r.error.code).toBe("bad_channels");
  });

  it("rejects a bitrate on a lossless format", () => {
    const r = planExport(preset({ format: "flac", bitrate_kbps: 192 }));
    expect("error" in r && r.error.code).toBe("bitrate_on_lossless");
  });

  it("rejects a missing bitrate on a lossy format", () => {
    const r = planExport(preset({ format: "mp3", bitrate_kbps: null }));
    expect("error" in r && r.error.code).toBe("missing_bitrate");
  });

  it("rejects an out-of-range bitrate either side", () => {
    const low = planExport(preset({ bitrate_kbps: MIN_BITRATE_KBPS - 1 }));
    expect("error" in low && low.error.code).toBe("bitrate_out_of_range");
    const high = planExport(preset({ bitrate_kbps: MAX_BITRATE_KBPS + 1 }));
    expect("error" in high && high.error.code).toBe("bitrate_out_of_range");
  });

  it("accepts the bitrate boundaries inclusively", () => {
    expect(
      "plan" in planExport(preset({ bitrate_kbps: MIN_BITRATE_KBPS })),
    ).toBe(true);
    expect(
      "plan" in planExport(preset({ bitrate_kbps: MAX_BITRATE_KBPS })),
    ).toBe(true);
  });
});

describe("export plan: ffmpeg args", () => {
  it("builds an MP3 arg vector in the documented order", () => {
    const plan = planOf(planExport(preset(), 48000));
    const args = buildFfmpegArgs(plan, "/tmp/master.wav", "/out/show.mp3");
    expect(args).toEqual([
      "-y",
      "-i",
      "/tmp/master.wav",
      "-c:a",
      "libmp3lame",
      "-b:a",
      "192k",
      "-ar",
      "48000",
      "-ac",
      "2",
      "/out/show.mp3",
    ]);
  });

  it("omits -b:a and adds compression level for FLAC", () => {
    const plan = planOf(
      planExport(preset({ format: "flac", bitrate_kbps: null }), 44100),
    );
    const args = buildFfmpegArgs(plan, "/tmp/m.wav", "/out/a.flac");
    expect(args).not.toContain("-b:a");
    expect(args).toContain("-compression_level");
    expect(args[args.indexOf("-compression_level") + 1]).toBe("8");
    expect(args[args.indexOf("-ar") + 1]).toBe("44100");
    expect(args[args.indexOf("-ac") + 1]).toBe("2"); // default preset is stereo
    expect(args.at(-1)).toBe("/out/a.flac");
  });

  it("encodes AAC mono correctly", () => {
    const plan = planOf(
      planExport(preset({ format: "aac", bitrate_kbps: 128, channels: 1 })),
    );
    const args = buildFfmpegArgs(plan, "in.wav", "out.m4a");
    expect(args).toContain("aac");
    expect(args[args.indexOf("-b:a") + 1]).toBe("128k");
    expect(args[args.indexOf("-ac") + 1]).toBe("1");
  });

  it("throws for a native WAV plan (no encode step)", () => {
    const plan = planOf(
      planExport(preset({ format: "wav", bitrate_kbps: null })),
    );
    expect(() => buildFfmpegArgs(plan, "in.wav", "out.wav")).toThrow(
      /natively/,
    );
  });

  it("always starts with -y -i <input> and ends with <output>", () => {
    const plan = planOf(planExport(preset()));
    const args = buildFfmpegArgs(plan, "INPUT", "OUTPUT");
    expect(args.slice(0, 3)).toEqual(["-y", "-i", "INPUT"]);
    expect(args.at(-1)).toBe("OUTPUT");
  });
});

describe("export plan: human summary", () => {
  it("summarises a lossy stereo plan", () => {
    const plan = planOf(planExport(preset(), 48000));
    expect(describePlan(plan)).toBe(
      "192 kbps MP3, stereo, 48 kHz (ffmpeg encode)",
    );
  });

  it("summarises a native WAV plan with bit depth", () => {
    const plan = planOf(
      planExport(
        preset({ format: "wav", bitrate_kbps: null, channels: 1 }),
        44100,
      ),
    );
    expect(describePlan(plan)).toBe("24-bit WAV, mono, 44.1 kHz (native)");
  });

  it("summarises a FLAC plan without a bitrate", () => {
    const plan = planOf(
      planExport(preset({ format: "flac", bitrate_kbps: null, channels: 2 })),
    );
    expect(describePlan(plan)).toBe("FLAC, stereo, 48 kHz (ffmpeg encode)");
  });
});
