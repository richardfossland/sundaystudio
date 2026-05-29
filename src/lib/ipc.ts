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
  ToneResult,
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
};

/** Bundled namespace for ergonomic imports. */
export const ipc = { app, audio };
