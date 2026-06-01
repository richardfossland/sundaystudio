/**
 * Jingle pipeline — offline, pure TypeScript.
 *
 * `JingleSpec` describes a jingle to produce; `jingle_render_plan` builds the
 * ffmpeg argument list that assembles instrument stems + optional voiceover into
 * a finished jingle WAV without any network calls.
 *
 * The online part (Suno music generation, AI voiceover synthesis) lives in
 * `src-tauri/src/ai/` and is gated to Pro + hardware / network. This module is
 * the testable offline layer that decides *how* to render once the stems exist.
 */

// ── Types ───────────────────────────────────────────────────────────────────

export type JingleDuration = 20 | 30 | 60;
export type JingleMood = "energetic" | "calm" | "worshipful" | "professional";

/**
 * Everything a user fills out to describe the jingle they want.
 *
 * `instruments` is a list of stem names that will be mixed together
 * (e.g. `["piano", "strings", "drums"]`). The render plan maps each name to a
 * relative file path inside the project's `jingles/stems/` folder.
 */
export interface JingleSpec {
  title: string;
  duration_sec: JingleDuration;
  mood: JingleMood;
  tempo_bpm: number;
  instruments: string[];
  voiceover_text?: string;
}

/** A single stem track: the source WAV and how to mix it in. */
export interface StemTrack {
  /** Relative path inside the project's stems directory. */
  path: string;
  /** Mix gain in dB (0 = unity, negative = quiet, positive = boost). */
  gain_db: number;
  /** Trim the stem to the jingle duration (true by default). */
  trim: boolean;
}

/**
 * The ffmpeg render plan for a jingle.
 *
 * `ffmpeg_args` is the full argument list (excluding the `ffmpeg` binary itself)
 * that will produce `output_path` from the stems. It is designed for
 * `ffmpeg_sidecar` / `std::process::Command`.
 */
export interface JingleRenderPlan {
  spec: JingleSpec;
  stems: StemTrack[];
  /** Relative output path within the project dir, e.g. `jingles/my-jingle.wav`. */
  output_path: string;
  ffmpeg_args: string[];
  /** Human-readable summary of the plan. */
  description: string;
}

/** Validation result — either ok or an array of field-level errors. */
export type ValidationResult =
  | { ok: true }
  | { ok: false; errors: ValidationError[] };

export interface ValidationError {
  field: keyof JingleSpec | "instruments.length";
  message: string;
  code: string;
}

// ── Constants ───────────────────────────────────────────────────────────────

export const VALID_DURATIONS: JingleDuration[] = [20, 30, 60];
export const VALID_MOODS: JingleMood[] = [
  "energetic",
  "calm",
  "worshipful",
  "professional",
];
export const MIN_BPM = 60;
export const MAX_BPM = 200;
export const MAX_INSTRUMENTS = 8;

// ── Validation ───────────────────────────────────────────────────────────────

/**
 * Validate a `JingleSpec`. Returns `{ ok: true }` when the spec is well-formed,
 * or `{ ok: false, errors }` with one error per failing field.
 *
 * Pure and synchronous — no I/O, no IPC.
 */
export function validateJingleSpec(spec: JingleSpec): ValidationResult {
  const errors: ValidationError[] = [];

  // title
  if (!spec.title || spec.title.trim().length === 0) {
    errors.push({
      field: "title",
      code: "title_required",
      message: "Title is required.",
    });
  }

  // duration_sec
  if (!VALID_DURATIONS.includes(spec.duration_sec)) {
    errors.push({
      field: "duration_sec",
      code: "invalid_duration",
      message: `Duration must be one of ${VALID_DURATIONS.join(", ")} seconds.`,
    });
  }

  // mood
  if (!VALID_MOODS.includes(spec.mood)) {
    errors.push({
      field: "mood",
      code: "invalid_mood",
      message: `Mood must be one of: ${VALID_MOODS.join(", ")}.`,
    });
  }

  // tempo_bpm
  if (!Number.isFinite(spec.tempo_bpm)) {
    errors.push({
      field: "tempo_bpm",
      code: "bpm_not_finite",
      message: "BPM must be a finite number.",
    });
  } else if (spec.tempo_bpm < MIN_BPM || spec.tempo_bpm > MAX_BPM) {
    errors.push({
      field: "tempo_bpm",
      code: "bpm_out_of_range",
      message: `BPM must be between ${MIN_BPM} and ${MAX_BPM}.`,
    });
  } else if (!Number.isInteger(spec.tempo_bpm)) {
    errors.push({
      field: "tempo_bpm",
      code: "bpm_not_integer",
      message: "BPM must be a whole number.",
    });
  }

  // instruments
  if (!Array.isArray(spec.instruments) || spec.instruments.length === 0) {
    errors.push({
      field: "instruments",
      code: "instruments_required",
      message: "At least one instrument stem is required.",
    });
  } else if (spec.instruments.length > MAX_INSTRUMENTS) {
    errors.push({
      field: "instruments.length",
      code: "too_many_instruments",
      message: `Maximum ${MAX_INSTRUMENTS} instrument stems allowed.`,
    });
  }

  return errors.length === 0 ? { ok: true } : { ok: false, errors };
}

// ── Render plan ─────────────────────────────────────────────────────────────

/**
 * Build a `JingleRenderPlan` from a validated `JingleSpec`.
 *
 * The plan mixes all instrument stems into a single output WAV using ffmpeg's
 * `amix` filter, optionally prepending a voiceover track with a short fade.
 *
 * Assumptions:
 *   - Each stem WAV lives at `stems/<instrument_name>.wav` relative to the
 *     project's jingles folder (populated by the Suno download step).
 *   - The voiceover WAV (if present) lives at `stems/voiceover.wav`.
 *   - All stems and voiceover are at 48 kHz / 16-bit / stereo.
 *   - Output is a 24-bit 48 kHz stereo WAV trimmed to `duration_sec`.
 *
 * This function is pure: same inputs → same outputs, no side effects.
 */
export function jingle_render_plan(
  spec: JingleSpec,
  /** Override the project-relative output path for tests. */
  outputPath?: string,
): JingleRenderPlan {
  const stems: StemTrack[] = spec.instruments.map((instrument) => {
    const gain = moodGain(instrument, spec.mood);
    return {
      path: `stems/${sanitizeStemName(instrument)}.wav`,
      gain_db: gain,
      trim: true,
    };
  });

  const hasVoiceover = Boolean(spec.voiceover_text?.trim());
  if (hasVoiceover) {
    stems.push({
      path: "stems/voiceover.wav",
      gain_db: 0,
      trim: false, // voiceover may be shorter than the music bed
    });
  }

  const resolvedOutput =
    outputPath ??
    `jingles/${sanitizeStemName(spec.title)}_${spec.duration_sec}s.wav`;

  const ffmpeg_args = buildJingleFfmpegArgs(stems, resolvedOutput, spec);

  const instrList = spec.instruments.join(", ");
  const voiceNote = hasVoiceover ? " + voiceover" : "";
  const description =
    `${spec.title} · ${spec.duration_sec}s · ${spec.mood} · ${spec.tempo_bpm} BPM · ` +
    `${instrList}${voiceNote} → ${resolvedOutput}`;

  return { spec, stems, output_path: resolvedOutput, ffmpeg_args, description };
}

// ── ffmpeg arg builder ───────────────────────────────────────────────────────

/**
 * Build the ffmpeg argument list for a jingle render.
 *
 * Strategy:
 *   1. One `-i` per stem.
 *   2. `volume` filter per stem (converts gain_db to a linear multiplier).
 *   3. `amix` to sum all music-bed stems (not the voiceover).
 *   4. If voiceover is present: `amix` music bed + voiceover at reduced
 *      music bed level (−6 dB under voice).
 *   5. `atrim` to clamp the output to `duration_sec`.
 *   6. `aformat` for 24-bit 48 kHz stereo PCM.
 *   7. Output path.
 */
function buildJingleFfmpegArgs(
  stems: StemTrack[],
  outputPath: string,
  spec: JingleSpec,
): string[] {
  const args: string[] = ["-y"];

  // Input files
  stems.forEach((s) => {
    args.push("-i", s.path);
  });

  const musicStems = stems.filter((s) => s.path !== "stems/voiceover.wav");
  const hasVoiceover = stems.some((s) => s.path === "stems/voiceover.wav");

  // Build filter graph
  const filterParts: string[] = [];

  // Per-stem volume filters
  stems.forEach((stem, idx) => {
    const linear = dbToLinear(stem.gain_db);
    filterParts.push(`[${idx}:a]volume=${linear.toFixed(4)}[s${idx}]`);
  });

  // Mix music stems together
  const musicLabels = musicStems
    .map((_, i) => `[s${stems.indexOf(musicStems[i])}]`)
    .join("");

  if (musicStems.length > 1) {
    filterParts.push(
      `${musicLabels}amix=inputs=${musicStems.length}:duration=longest:normalize=0[music]`,
    );
  } else {
    // Only one music stem — just rename the label
    filterParts.push(`${musicLabels}aresample=48000[music]`);
  }

  // Trim music to duration_sec
  filterParts.push(
    `[music]atrim=0:${spec.duration_sec},asetpts=PTS-STARTPTS[musicT]`,
  );

  let finalLabel = "[musicT]";

  if (hasVoiceover) {
    const voIdx = stems.findIndex((s) => s.path === "stems/voiceover.wav");
    // Duck the music bed by 6 dB under the voiceover
    filterParts.push(`[musicT]volume=0.5[musicDucked]`);
    filterParts.push(
      `[musicDucked][s${voIdx}]amix=inputs=2:duration=first:normalize=0[withVO]`,
    );
    finalLabel = "[withVO]";
  }

  // Final format: 24-bit / 48 kHz / stereo
  filterParts.push(
    `${finalLabel}aformat=sample_fmts=s32:sample_rates=48000:channel_layouts=stereo[out]`,
  );

  args.push("-filter_complex", filterParts.join("; "));
  args.push("-map", "[out]");
  args.push("-c:a", "pcm_s24le");
  args.push("-ar", "48000");
  args.push("-ac", "2");
  args.push(outputPath);

  return args;
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/** Convert dB to a linear amplitude multiplier (0 dB → 1.0). */
export function dbToLinear(db: number): number {
  return Math.pow(10, db / 20);
}

/**
 * Suggest a mix gain in dB for an instrument based on the jingle mood.
 * Purely heuristic — the Suno-generated stems already target the right energy
 * level; we just give gentle nudges so the final mix feels right.
 */
function moodGain(instrument: string, mood: JingleMood): number {
  const lower = instrument.toLowerCase();
  const boostPercussion = mood === "energetic" ? 2 : 0;
  const quietPercussion = mood === "calm" || mood === "worshipful" ? -3 : 0;

  if (lower.includes("drum") || lower.includes("perc")) {
    return boostPercussion + quietPercussion;
  }
  if (lower.includes("bass")) {
    return mood === "calm" ? -2 : 0;
  }
  if (lower.includes("lead") || lower.includes("synth")) {
    return mood === "professional" ? -1 : 1;
  }
  return 0; // unity for everything else
}

/** Sanitize a stem or title name to be filesystem-safe. */
export function sanitizeStemName(name: string): string {
  return (
    name
      .toLowerCase()
      .trim()
      .replace(/[^a-z0-9_-]/g, "_")
      .replace(/_+/g, "_")
      .replace(/^_|_$/g, "") || "untitled"
  );
}
