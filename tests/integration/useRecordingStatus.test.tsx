/**
 * Tests for the recording-status hook and its pure banner-decision logic.
 *
 * Tauri's `invoke` is mocked so the hook can be exercised with synthetic
 * `RecordingStatus` payloads (writer-failed true/false, dropped > 0) — no audio
 * hardware involved. The pure `deriveRecordingAlerts` helper is tested directly;
 * the React hook is rendered with a real QueryClient and the mocked IPC.
 */

import { describe, it, expect, vi, beforeEach } from "vitest";
import { renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { createElement } from "react";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import type { RecordingStatus } from "@/lib/bindings";
import {
  deriveRecordingAlerts,
  useRecordingStatus,
} from "@/lib/useRecordingStatus";

function status(overrides: Partial<RecordingStatus> = {}): RecordingStatus {
  return {
    recording: false,
    captured_frames: 0,
    duration_ms: 0,
    dropped: 0,
    meters_dbfs: [],
    writer_failed: false,
    ...overrides,
  };
}

describe("deriveRecordingAlerts (pure banner logic)", () => {
  it("is all-clear for a null status", () => {
    expect(deriveRecordingAlerts(null)).toEqual({
      recording: false,
      writerFailed: false,
      hasDropped: false,
      dropped: 0,
    });
  });

  it("is all-clear for a healthy live take", () => {
    const a = deriveRecordingAlerts(
      status({ recording: true, captured_frames: 4800, dropped: 0 }),
    );
    expect(a.recording).toBe(true);
    expect(a.writerFailed).toBe(false);
    expect(a.hasDropped).toBe(false);
  });

  it("raises writerFailed when the writer thread died mid-take", () => {
    const a = deriveRecordingAlerts(
      status({ recording: true, writer_failed: true }),
    );
    expect(a.writerFailed).toBe(true);
  });

  it("raises hasDropped and reports the count when dropped > 0", () => {
    const a = deriveRecordingAlerts(status({ recording: true, dropped: 12 }));
    expect(a.hasDropped).toBe(true);
    expect(a.dropped).toBe(12);
  });

  it("does not raise hasDropped at exactly zero dropped", () => {
    expect(deriveRecordingAlerts(status({ dropped: 0 })).hasDropped).toBe(
      false,
    );
  });

  it("coerces a non-finite dropped count to a safe zero", () => {
    const a = deriveRecordingAlerts(
      status({ dropped: Number.NaN as unknown as number }),
    );
    expect(a.dropped).toBe(0);
    expect(a.hasDropped).toBe(false);
  });

  it("never hides a writer failure even if reported while idle", () => {
    const a = deriveRecordingAlerts(
      status({ recording: false, writer_failed: true }),
    );
    expect(a.writerFailed).toBe(true);
  });
});

function wrapper() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
}

describe("useRecordingStatus (mocked IPC)", () => {
  beforeEach(() => invokeMock.mockReset());

  it("polls audio_record_status and surfaces a writer failure", async () => {
    invokeMock.mockResolvedValue(
      status({ recording: true, writer_failed: true, dropped: 0 }),
    );
    const { result } = renderHook(() => useRecordingStatus(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.alerts.writerFailed).toBe(true));
    expect(invokeMock).toHaveBeenCalledWith("audio_record_status", undefined);
    expect(result.current.alerts.hasDropped).toBe(false);
  });

  it("surfaces dropped samples as a warning with the count", async () => {
    invokeMock.mockResolvedValue(status({ recording: true, dropped: 7 }));
    const { result } = renderHook(() => useRecordingStatus(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.alerts.hasDropped).toBe(true));
    expect(result.current.alerts.dropped).toBe(7);
    expect(result.current.alerts.writerFailed).toBe(false);
  });

  it("reports an all-clear idle status without raising any alert", async () => {
    invokeMock.mockResolvedValue(status());
    const { result } = renderHook(() => useRecordingStatus(), {
      wrapper: wrapper(),
    });
    await waitFor(() => expect(result.current.status).not.toBeNull());
    expect(result.current.alerts.writerFailed).toBe(false);
    expect(result.current.alerts.hasDropped).toBe(false);
    expect(result.current.alerts.recording).toBe(false);
  });

  it("does not poll when disabled", () => {
    invokeMock.mockResolvedValue(status());
    const { result } = renderHook(
      () => useRecordingStatus({ enabled: false }),
      { wrapper: wrapper() },
    );
    expect(invokeMock).not.toHaveBeenCalled();
    expect(result.current.status).toBeNull();
    expect(result.current.alerts.writerFailed).toBe(false);
  });
});
