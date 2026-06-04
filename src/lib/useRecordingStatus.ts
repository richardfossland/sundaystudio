/**
 * `useRecordingStatus` — polls the live recording transport and derives the
 * safety flags the UI must surface immediately ("recording is sacred").
 *
 * Polling goes through TanStack Query (the same pattern the settings /
 * diagnostics screens use). `ipc.audio.recordStatus()` is safe to poll without
 * audio hardware: it returns the idle status when nothing is rolling, so the
 * hook is harmless when no take is live and lights up the banners the instant a
 * disk-write failure or ring overrun appears during a real capture.
 *
 * The banner-decision logic is factored into the pure `deriveRecordingAlerts`
 * helper so it can be unit-tested with a mocked status (no React, no hardware).
 */

import { useQuery } from "@tanstack/react-query";

import { ipc } from "./ipc";
import type { RecordingStatus } from "./bindings";

/** How often the transport state is polled, in ms (≈4Hz — enough to surface a
 *  writer failure quickly without hammering the IPC bridge). */
export const RECORDING_POLL_MS = 250;

/** The pure, testable verdict on a `RecordingStatus`: which safety surfaces the
 *  UI should show. Derived from the raw status so the banner/badge logic is one
 *  decision point shared by the component and its tests. */
export interface RecordingAlerts {
  /** A take is actively being captured. */
  recording: boolean;
  /** The writer thread died mid-take — show the prominent red error banner.
   *  Capture may still be running, but nothing is reaching disk. */
  writerFailed: boolean;
  /** Samples were lost to ring overruns — show the warning badge. */
  hasDropped: boolean;
  /** How many samples were dropped (for the badge label). */
  dropped: number;
}

/** Derive the UI safety verdict from a (possibly null) recording status. Pure:
 *  no IPC, no React — the single source of truth for both the banner component
 *  and its unit tests. A null/idle status produces an all-clear verdict. */
export function deriveRecordingAlerts(
  status: RecordingStatus | null | undefined,
): RecordingAlerts {
  if (!status) {
    return {
      recording: false,
      writerFailed: false,
      hasDropped: false,
      dropped: 0,
    };
  }
  const dropped = Number.isFinite(status.dropped) ? status.dropped : 0;
  return {
    recording: status.recording,
    // Honour the backend flag even if it somehow reports while idle — a disk
    // failure is never something to hide.
    writerFailed: status.writer_failed,
    hasDropped: dropped > 0,
    dropped,
  };
}

export interface UseRecordingStatusOptions {
  /** Poll only while enabled (e.g. while the transport is armed/rolling).
   *  Defaults to `true` — polling idle is cheap and returns the idle status. */
  enabled?: boolean;
  /** Override the poll interval (ms). */
  pollMs?: number;
}

/** Poll the recording transport and return the raw status plus the derived
 *  safety alerts. Designed to be dropped into the recording page header. */
export function useRecordingStatus(options: UseRecordingStatusOptions = {}) {
  const { enabled = true, pollMs = RECORDING_POLL_MS } = options;

  const query = useQuery<RecordingStatus>({
    queryKey: ["audio_record_status"],
    queryFn: ipc.audio.recordStatus,
    enabled,
    refetchInterval: enabled ? pollMs : false,
    // Status is real-time; never serve a stale cached value.
    staleTime: 0,
    // The transport is hardware-only; a failed poll outside Tauri should not
    // spam retries — the idle fallback in `deriveRecordingAlerts` covers it.
    retry: false,
  });

  return {
    status: query.data ?? null,
    alerts: deriveRecordingAlerts(query.data),
    isLoading: query.isLoading,
    error: query.error as Error | null,
  };
}
