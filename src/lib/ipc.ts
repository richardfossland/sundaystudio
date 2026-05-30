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

import type {
  AppError,
  AppInfo,
  AudioDeviceList,
  AudioSettings,
  ExportPresetInfo,
  ExportResult,
  LatencyEstimate,
  LoudnessMeasurement,
  LoudnessTarget,
  Marker,
  MasterPresetInfo,
  PresetInfo,
  Project,
  ProjectSnapshot,
  RecentProject,
  Region,
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

// ── Export ───────────────────────────────────────────────────────────────────

export const exporter = {
  /** Platform-ready export presets (format + bitrate + channels + LUFS target). */
  presets: () => call<ExportPresetInfo[]>("export_presets"),
  /** Bounce the open project's latest take to a mastered, normalised WAV. */
  render: (presetId: string, masterPresetId?: string) =>
    call<ExportResult>("export_render", { presetId, masterPresetId }),
};

/** Bundled namespace for ergonomic imports. */
export const ipc = { app, audio, project, dsp, edit, exporter };
