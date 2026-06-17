/**
 * Typed wrappers around Tauri's `invoke()`.
 *
 * One function per Rust command. Wraps `invoke<T>(name, args)` so:
 *   - The TypeScript caller has a stable signature
 *   - Rust `AppError` is rethrown as a JS `IPCError` the React code can catch
 *   - Dev-mode logs every call for debugging (toggle via `VITE_IPC_LOG`)
 *
 * Convention: command names are `entity_verb` (e.g. `app_info`, `audio_devices`).
 * Matches `commands::*` in Rust.
 */

import { invoke } from "@tauri-apps/api/core";

import type { JingleSpec } from "./jingle";

import type {
  AppError,
  AppInfo,
  AudioDeviceList,
  AudioSettings,
  ExportChapterInput,
  ExportPresetInfo,
  ExportResult,
  ImportRequest,
  JingleResult,
  LatencyEstimate,
  LevelingResult,
  LoudnessMeasurement,
  LoudnessTarget,
  Marker,
  MasterPresetInfo,
  PlaybackStatus,
  PresetInfo,
  Project,
  ProjectMeta,
  ProjectSnapshot,
  RecentProject,
  RecordingStatus,
  Region,
  ShowNotes,
  ShowNotesInput,
  SilenceSpan,
  TemplateInfo,
  TimelineSnapshot,
  ToneResult,
  Track,
  WaveformPeaks,
} from "./bindings";

const DEV = import.meta.env.DEV;
const LOG_IPC = DEV && import.meta.env.VITE_IPC_LOG !== "false";

/** Wrapper around Tauri's error that preserves the Rust `code` field. */
export class IPCError extends Error {
  readonly code: AppError["code"];
  constructor(err: AppError, options?: ErrorOptions) {
    super(err.message, options);
    this.code = err.code;
    this.name = "IPCError";
  }
}

/** Map a raw value thrown by `invoke()` into a typed error. Pure + testable:
 *  Tauri rethrows a serialised `AppError` as a plain `{ code, message }`. */
export function toIPCError(raw: unknown): Error {
  if (raw && typeof raw === "object" && "code" in raw && "message" in raw) {
    return new IPCError(raw as AppError, { cause: raw });
  }
  if (raw instanceof Error) return raw;
  return new Error(String(raw), { cause: raw });
}

/** Human-readable message for any thrown value — the single source of truth
 *  for the `e instanceof Error ? e.message : String(e)` pattern the feature
 *  pages repeated by hand. Pure + testable. */
export function errorMessage(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && "message" in err) {
    const m = (err as { message: unknown }).message;
    if (typeof m === "string") return m;
  }
  return String(err);
}

async function call<T>(
  cmd: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (LOG_IPC) console.debug(`[ipc] → ${cmd}`, args);
  try {
    const out = await invoke<T>(cmd, args);
    if (LOG_IPC) console.debug(`[ipc] ← ${cmd}`, out);
    return out;
  } catch (raw) {
    throw toIPCError(raw);
  }
}

// ── App ────────────────────────────────────────────────────────────────────

export const app = {
  /** "Hello SundayStudio" — proves the Rust ↔ React bridge works. */
  info: () => call<AppInfo>("app_info"),
};

// ── Audio ────────────────────────────────────────────────────────────────────

export const audio = {
  /** Enumerate input/output devices on the default host (cpal). */
  devices: () => call<AudioDeviceList>("audio_devices"),
  /** Write a 1-second 440 Hz sine WAV to the OS temp dir (hound smoke test). */
  recordTestTone: () => call<ToneResult>("audio_record_test_tone"),
  /** Load persisted audio settings (defaults on first run). */
  getSettings: () => call<AudioSettings>("audio_get_settings"),
  /** Persist audio settings (validated backend-side). */
  setSettings: (newSettings: AudioSettings) =>
    call<void>("audio_set_settings", { newSettings }),
  /** Estimate round-trip monitoring latency for a sample-rate/buffer choice. */
  latencyEstimate: (sampleRate: number, bufferSize: number) =>
    call<LatencyEstimate>("audio_latency_estimate", {
      sampleRate,
      bufferSize,
    }),

  // ── Transport: live capture (Phase 2.2) ──────────────────────────────────
  //
  // The recorder owns real hardware (cpal input stream + writer thread), so
  // start/stop can only be exercised on a real audio rig — they are wired here
  // but hardware-gated. `recordStatus` is safe to poll without hardware: it
  // returns the idle status when nothing is rolling, which is what the
  // `useRecordingStatus` hook surfaces (writer-failed / dropped warnings).

  /** Start capturing the open project's tracks. Resolves the input device
   *  (omit `deviceName` for the host default) and arms one capture track per
   *  project track. Needs real audio hardware. */
  recordStart: (deviceName?: string, channels?: number) =>
    call<RecordingStatus>("audio_record_start", { deviceName, channels }),
  /** Stop the live take, finalise the WAVs and lay them on the timeline.
   *  Returns the refreshed timeline. Needs an active take. */
  recordStop: () => call<TimelineSnapshot>("audio_record_stop"),
  /** Poll the live recording state: rolling?, captured duration, dropped
   *  samples (overruns), per-channel meters, and — critically — whether the
   *  writer thread died mid-take (a disk-write failure). Returns the idle
   *  status when nothing is recording, so this is safe to poll always. */
  recordStatus: () => call<RecordingStatus>("audio_record_status"),
};

// ── Transport: timeline playback (Phase 3) ───────────────────────────────────
//
// Plumbing only: these wrap the playback render-thread commands. The render
// thread drives a cpal *output* stream, so playback can only be verified on
// real hardware — wired, not verified.

export const transport = {
  /** Start playing the open project's timeline from the start. Needs an output
   *  device (hardware-gated). */
  play: () => call<PlaybackStatus>("audio_play_timeline"),
  /** Resume the current (paused) playback session. */
  resume: () => call<void>("audio_play"),
  /** Pause playback, holding the playhead. */
  pause: () => call<void>("audio_pause"),
  /** Seek the playhead to a millisecond position (clamped to the length). */
  seek: (positionMs: number) => call<void>("audio_seek", { positionMs }),
  /** Mute/unmute a timeline track during playback (by resolved track index). */
  muteTrack: (trackIdx: number, muted: boolean) =>
    call<void>("audio_playback_mute", { trackIdx, muted }),
  /** The current transport state (poll ~60fps to draw the playhead). */
  status: () => call<PlaybackStatus>("audio_playback_status"),
  /** Stop and tear down the playback session (no-op when nothing plays). */
  stop: () => call<void>("audio_stop_playback"),
};

// ── Project ──────────────────────────────────────────────────────────────────

export const project = {
  /** Create a new `.scast` project at `path` and make it current. */
  create: (
    path: string,
    name: string,
    sampleRate: number,
    channelCount: number,
  ) =>
    call<ProjectSnapshot>("project_create", {
      path,
      name,
      sampleRate,
      channelCount,
    }),
  /** The quick-start templates for the gallery. */
  templates: () => call<TemplateInfo[]>("project_templates"),
  /** Create a project pre-configured from a quick-start template. */
  createFromTemplate: (path: string, name: string, templateId: string) =>
    call<ProjectSnapshot>("project_create_from_template", {
      path,
      name,
      templateId,
    }),
  /** Open an existing project and make it current. */
  open: (path: string) => call<ProjectSnapshot>("project_open", { path }),
  /** Recent projects, most-recent first. */
  recent: () => call<RecentProject[]>("project_recent"),
  /** Reload the current project (project + tracks + markers). */
  snapshot: () => call<ProjectSnapshot>("project_snapshot"),
  /** Rename the current project. */
  rename: (name: string) => call<Project>("project_rename", { name }),
  /** Back up the current project's database; returns the backup path. */
  backup: () => call<string>("project_backup"),
  /** Add a track to the current project. */
  addTrack: (name: string, color: string) =>
    call<Track>("track_add", { name, color }),
  /** Persist a track's state. */
  updateTrack: (track: Track) => call<void>("track_update", { track }),
  /** Delete a track. */
  deleteTrack: (id: string) => call<void>("track_delete", { id }),
  /** Add a marker / chapter. */
  addMarker: (positionMs: number, label: string, color: string) =>
    call<Marker>("marker_add", { positionMs, label, color }),
  /** Delete a marker. */
  deleteMarker: (id: string) => call<void>("marker_delete", { id }),

  // ── Phase 2.1 registry CRUD ───────────────────────────────────────────────

  /** Create a new project in the app data dir (no file dialog) and register it.
   *  Returns the registry entry. Use `project.snapshot()` to get full details. */
  new: (name: string) => call<ProjectMeta>("project_new", { name }),

  /** Persist a project's mutable metadata (name) to the registry.
   *  Call after renaming to keep the registry entry in sync. */
  save: (registryId: string, name: string) =>
    call<ProjectMeta>("project_save", { registryId, name }),

  /** Open a registered project by its registry id and make it current.
   *  Returns the registry entry; follow with `project.snapshot()` for details. */
  load: (registryId: string) =>
    call<ProjectMeta>("project_load", { registryId }),

  /** List all registered projects, most-recently created first. */
  list: () => call<ProjectMeta[]>("project_list"),

  /** Remove a project from the registry (the .scast folder is kept on disk). */
  delete: (registryId: string) => call<void>("project_delete", { registryId }),
};

// ── DSP ──────────────────────────────────────────────────────────────────────

export const dsp = {
  /** Bundled voice-processing factory presets. */
  presets: () => call<PresetInfo[]>("dsp_presets"),
  /** "Match to platform" loudness targets (Spotify / Apple / YouTube / EBU). */
  loudnessTargets: () => call<LoudnessTarget[]>("dsp_loudness_targets"),
  /** Bundled mastering presets (master chain + platform target). */
  masterPresets: () => call<MasterPresetInfo[]>("dsp_master_presets"),
  /** Measure a WAV's loudness (integrated/short/momentary LUFS + true peak). */
  analyzeFile: (path: string) =>
    call<LoudnessMeasurement>("dsp_analyze_file", { path }),
};

// ── Edit / timeline ────────────────────────────────────────────────────────────

export const edit = {
  /** The open project's takes and placed regions (the editor timeline). */
  timeline: () => call<TimelineSnapshot>("project_timeline"),
  /** A take track's waveform overview (computed + cached on first request). */
  peaks: (takeId: string, sourceTrackId: string) =>
    call<WaveformPeaks>("audio_peaks", { takeId, sourceTrackId }),
  /** Detect silent gaps (take-relative ms) in a take track for trimming. */
  analyzeSilence: (
    takeId: string,
    sourceTrackId: string,
    thresholdDb: number,
    minSilenceMs: number,
  ) =>
    call<SilenceSpan[]>("analyze_silence", {
      takeId,
      sourceTrackId,
      thresholdDb,
      minSilenceMs,
    }),
  /** Place a new region on a track. */
  addRegion: (
    takeId: string,
    sourceTrackId: string,
    targetTrackId: string,
    startInTakeMs: number,
    endInTakeMs: number,
    positionInTimelineMs: number,
  ) =>
    call<Region>("region_add", {
      takeId,
      sourceTrackId,
      targetTrackId,
      startInTakeMs,
      endInTakeMs,
      positionInTimelineMs,
    }),
  /** Insert a region with a caller-supplied id (for split / undo-redo). */
  createRegion: (region: Region) => call<Region>("region_create", { region }),
  /** Persist a region edit (move / trim / fade / gain). */
  updateRegion: (region: Region) => call<void>("region_update", { region }),
  /** Delete a region (its take is untouched). */
  deleteRegion: (id: string) => call<void>("region_delete", { id }),
  /** Import existing WAVs onto the timeline as a new take. */
  importTakes: (paths: string[]) =>
    call<TimelineSnapshot>("take_import", { paths }),
};

// ── Deep links (Rec → Studio import handoff) ─────────────────────────────────
//
// SundayRec hands a finished recording to SundayStudio by launching us with a
// `sundaystudio://import?path=…&returnTo=sundayrec` URL. The OS single-instance
// auto-registration of that scheme needs the bundled app + a real OS open-url
// event, so it can't be exercised headlessly — but `parseImport` is the pure
// validation step, reachable now via a pasted link on the diagnostics screen.

export const deeplink = {
  /** Validate + structure a `sundaystudio://import?…` URL into an
   *  `ImportRequest`. Throws a `validation` IPCError for a malformed link or a
   *  missing `path`. Pure backend logic — safe to call without hardware. */
  parseImport: (url: string) =>
    call<ImportRequest>("deeplink_parse_import", { url }),
};

// ── Export ───────────────────────────────────────────────────────────────────

export const exporter = {
  /** Platform-ready export presets (format + bitrate + channels + LUFS target). */
  presets: () => call<ExportPresetInfo[]>("export_presets"),
  /** Bounce the open project's latest take to a mastered, normalised WAV.
   *  Optional `chapters` (accepted AI/manual show-notes chapters) are embedded
   *  as ffmpeg chapter metadata in the encoded MP3/AAC/FLAC; they're ignored for
   *  a plain WAV bounce and when the ffmpeg sidecar is unavailable. */
  render: (
    presetId: string,
    masterPresetId?: string,
    chapters?: ExportChapterInput[],
  ) =>
    call<ExportResult>("export_render", { presetId, masterPresetId, chapters }),
};

// ── AI ───────────────────────────────────────────────────────────────────────

export const ai = {
  /** Ask Claude for per-track gain suggestions to balance the open project
   *  (Phase 5.1, Sunday Cast Pro). Needs `ANTHROPIC_API_KEY` on the backend;
   *  throws a `validation` IPCError when unavailable. Network I/O — runs on a
   *  blocking thread backend-side, never the audio thread. */
  autoLevel: () => call<LevelingResult>("ai_auto_level"),

  /** Generate a jingle from a spec via the music-generation wrapper (Phase 6,
   *  Sunday Cast Pro). Needs `SUNO_PROXY_URL` on the backend; throws a
   *  `validation` IPCError when unconfigured or when the spec is invalid.
   *  Network I/O — runs on a blocking thread backend-side, never the audio
   *  thread. Returns the generated audio's URL + metadata to download & mix. */
  generateJingle: (spec: JingleSpec) =>
    call<JingleResult>("ai_jingle_generate", { spec }),

  /** Generate show notes from a transcript (Phase 5.2, Sunday Cast Pro): title
   *  options, a Norwegian + English summary, timestamped chapters, tags, and a
   *  few suggested highlight clips. The transcript comes from the SundayRec
   *  deep-link caption handoff or a paste. Needs `ANTHROPIC_API_KEY` on the
   *  backend; throws a `validation` IPCError when unavailable, so the UI shows
   *  "legg til nøkkel for AI" and manual chapters keep working. The model only
   *  suggests — every field is sanitized backend-side. Network I/O runs on a
   *  blocking thread, never the audio thread. */
  showNotes: (input: ShowNotesInput) =>
    call<ShowNotes>("ai_show_notes", { input }),
};

/** Bundled namespace for ergonomic imports. */
export const ipc = {
  app,
  audio,
  transport,
  project,
  dsp,
  edit,
  exporter,
  ai,
  deeplink,
};
