/**
 * Pure export-plan reasoning (Phase 7.1, encode step) — turns an
 * `ExportPresetInfo` plus the target sample-rate into a validated *encode plan*
 * and the exact ffmpeg sidecar argument vector the renderer will hand the
 * bundled binary.
 *
 * The Rust renderer writes the master WAV natively (via `hound`); the lossy /
 * archival encode (MP3 / AAC / FLAC) is delegated to the ffmpeg sidecar. That
 * encode step is the same regardless of platform, so its argument-building is
 * cheap, deterministic, and worth unit-testing here *without* ffmpeg present:
 * the renderer just spawns the sidecar with the vector this module produces.
 *
 * Everything is pure — no IPC, no DOM, no spawning. The backend remains the
 * source of truth for the actual render; this validates the request and builds
 * the args so a bad combination (e.g. a bitrate on a lossless format) is caught
 * before we ever touch a process.
 *
 * NOTE: spawning the sidecar with these args is FFMPEG-SIDECAR-UNVERIFIED — the
 * binary is bundled in a later sub-phase. The arg *shape* is asserted here.
 */
import type { ExportFormat, ExportPresetInfo } from "./bindings";

/** Sample rates we allow for export, in Hz. 48 kHz is the podcast/broadcast
 *  default; 44.1 kHz is the legacy CD/MP3 rate some hosts still prefer. */
export const ALLOWED_SAMPLE_RATES = [44100, 48000] as const;
export type SampleRate = (typeof ALLOWED_SAMPLE_RATES)[number];
export const DEFAULT_SAMPLE_RATE: SampleRate = 48000;

/** Bitrate bounds (kbps) for lossy encodes. Below 64 sounds bad for voice;
 *  above 320 is wasteful and rejected by some hosts. */
export const MIN_BITRATE_KBPS = 64;
export const MAX_BITRATE_KBPS = 320;

/** PCM bit depth for the master WAV (24-bit is our archival default). */
export const WAV_BIT_DEPTH = 24;

/** Whether a format encodes losslessly (no bitrate; ffmpeg uses a quality/
 *  compression knob instead). WAV is native; FLAC is a lossless ffmpeg encode. */
export function isLossless(format: ExportFormat): boolean {
  return format === "wav" || format === "flac";
}

/** The ffmpeg audio codec for a format (the `-c:a` value). WAV is native and
 *  never goes through ffmpeg, but we surface its PCM codec for completeness. */
export function ffmpegCodec(format: ExportFormat): string {
  switch (format) {
    case "wav":
      return "pcm_s24le";
    case "mp3":
      return "libmp3lame";
    case "aac":
      return "aac";
    case "flac":
      return "flac";
  }
}

/** A validation problem with an export request, surfaced to the picker. */
export interface ExportPlanError {
  code:
    | "bad_channels"
    | "bad_sample_rate"
    | "bitrate_on_lossless"
    | "missing_bitrate"
    | "bitrate_out_of_range";
  message: string;
}

/**
 * A fully-resolved, validated encode plan: the effective parameters plus
 * whether the encode actually needs the ffmpeg sidecar (only WAV is native).
 */
export interface ExportPlan {
  format: ExportFormat;
  /** 1 = mono, 2 = stereo. */
  channels: number;
  sampleRate: SampleRate;
  /** kbps for lossy formats; null for WAV/FLAC. */
  bitrateKbps: number | null;
  /** ffmpeg `-c:a` codec name. */
  codec: string;
  /** True when the master WAV must be re-encoded by the sidecar. */
  requiresEncoder: boolean;
  /** File extension (no dot), matching the Rust `ExportFormat::extension`. */
  extension: string;
}

/** File extension (no dot) — mirrors Rust `ExportFormat::extension`. */
export function formatExtension(format: ExportFormat): string {
  switch (format) {
    case "wav":
      return "wav";
    case "mp3":
      return "mp3";
    case "aac":
      return "m4a";
    case "flac":
      return "flac";
  }
}

function isAllowedSampleRate(rate: number): rate is SampleRate {
  return (ALLOWED_SAMPLE_RATES as readonly number[]).includes(rate);
}

/**
 * Validate a preset + sample-rate combination into an `ExportPlan`, or return
 * the first problem found. Pure and total: every code path returns one or the
 * other. Catches the combinations that would make a downstream ffmpeg call
 * meaningless — a bitrate on a lossless format, a missing bitrate on a lossy
 * one, an out-of-range bitrate, a non-mono/stereo channel count, or an
 * unsupported sample rate.
 */
export function planExport(
  preset: ExportPresetInfo,
  sampleRate: number = DEFAULT_SAMPLE_RATE,
): { plan: ExportPlan } | { error: ExportPlanError } {
  if (preset.channels !== 1 && preset.channels !== 2) {
    return {
      error: {
        code: "bad_channels",
        message: `Channels must be 1 (mono) or 2 (stereo), got ${preset.channels}.`,
      },
    };
  }
  if (!isAllowedSampleRate(sampleRate)) {
    return {
      error: {
        code: "bad_sample_rate",
        message: `Sample rate must be one of ${ALLOWED_SAMPLE_RATES.join(
          " / ",
        )} Hz, got ${sampleRate}.`,
      },
    };
  }

  const lossless = isLossless(preset.format);
  if (lossless) {
    if (preset.bitrate_kbps != null) {
      return {
        error: {
          code: "bitrate_on_lossless",
          message: `${preset.format.toUpperCase()} is lossless and takes no bitrate (got ${
            preset.bitrate_kbps
          } kbps).`,
        },
      };
    }
  } else {
    if (preset.bitrate_kbps == null) {
      return {
        error: {
          code: "missing_bitrate",
          message: `${preset.format.toUpperCase()} needs a bitrate.`,
        },
      };
    }
    if (
      preset.bitrate_kbps < MIN_BITRATE_KBPS ||
      preset.bitrate_kbps > MAX_BITRATE_KBPS
    ) {
      return {
        error: {
          code: "bitrate_out_of_range",
          message: `Bitrate must be ${MIN_BITRATE_KBPS}–${MAX_BITRATE_KBPS} kbps, got ${preset.bitrate_kbps}.`,
        },
      };
    }
  }

  return {
    plan: {
      format: preset.format,
      channels: preset.channels,
      sampleRate,
      bitrateKbps: lossless ? null : preset.bitrate_kbps,
      codec: ffmpegCodec(preset.format),
      requiresEncoder: preset.format !== "wav",
      extension: formatExtension(preset.format),
    },
  };
}

/**
 * Build the ffmpeg sidecar argument vector that encodes the master WAV
 * (`inputPath`) into the plan's format at `outputPath`. Deterministic and
 * order-stable so tests can assert the exact vector.
 *
 * Shape: `-y -i <in> -c:a <codec> [-b:a <k>k] -ar <rate> -ac <ch> <out>`.
 *   - `-y` overwrites without prompting (the renderer manages temp paths).
 *   - `-b:a` is emitted only for lossy formats.
 *   - FLAC adds `-compression_level 8` (best ratio; encode time is irrelevant
 *     for an offline bounce).
 *
 * Throws for a WAV plan: WAV is written natively and never goes through the
 * sidecar, so asking for its args is a programming error.
 */
export function buildFfmpegArgs(
  plan: ExportPlan,
  inputPath: string,
  outputPath: string,
): string[] {
  if (plan.format === "wav") {
    throw new Error("WAV is written natively; it has no ffmpeg encode step.");
  }
  const args = ["-y", "-i", inputPath, "-c:a", plan.codec];
  if (plan.bitrateKbps != null) {
    args.push("-b:a", `${plan.bitrateKbps}k`);
  }
  if (plan.format === "flac") {
    args.push("-compression_level", "8");
  }
  args.push("-ar", String(plan.sampleRate), "-ac", String(plan.channels));
  args.push(outputPath);
  return args;
}

/**
 * A one-line, plain-language summary of a plan for the export confirm dialog,
 * e.g. "192 kbps MP3, stereo, 48 kHz (ffmpeg encode)" or
 * "24-bit WAV, mono, 44.1 kHz (native)". Pure and deterministic.
 */
export function describePlan(plan: ExportPlan): string {
  const ch = plan.channels === 1 ? "mono" : "stereo";
  const rate =
    plan.sampleRate % 1000 === 0
      ? `${plan.sampleRate / 1000} kHz`
      : `${(plan.sampleRate / 1000).toFixed(1)} kHz`;
  const stage = plan.requiresEncoder ? "ffmpeg encode" : "native";
  const head =
    plan.bitrateKbps != null
      ? `${plan.bitrateKbps} kbps ${plan.format.toUpperCase()}`
      : plan.format === "wav"
        ? `${WAV_BIT_DEPTH}-bit WAV`
        : plan.format.toUpperCase();
  return `${head}, ${ch}, ${rate} (${stage})`;
}
