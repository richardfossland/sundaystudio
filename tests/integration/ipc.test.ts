// Integration smoke — the IPC client layer. Mocks Tauri's `invoke` for the
// happy path, and tests the AppError -> IPCError mapping via the pure
// `toIPCError` helper (no async rejection plumbing required).
import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { ipc, toIPCError, IPCError } from "@/lib/ipc";

describe("ipc client", () => {
  beforeEach(() => invokeMock.mockReset());

  it("calls app_info and returns its result", async () => {
    invokeMock.mockResolvedValue({
      name: "SundayStudio",
      version: "0.1.0",
      tauri_version: "2",
      platform: "macos",
      arch: "aarch64",
      greeting: "hi",
    });
    const info = await ipc.app.info();
    expect(info.name).toBe("SundayStudio");
    expect(invokeMock).toHaveBeenCalledWith("app_info", undefined);
  });

  it("calls audio_devices and returns the device list", async () => {
    invokeMock.mockResolvedValue({
      host: "CoreAudio",
      inputs: [],
      outputs: [],
    });
    const list = await ipc.audio.devices();
    expect(list.host).toBe("CoreAudio");
    expect(invokeMock).toHaveBeenCalledWith("audio_devices", undefined);
  });

  it("polls audio_record_status with no args", async () => {
    invokeMock.mockResolvedValue({
      recording: false,
      captured_frames: 0,
      duration_ms: 0,
      dropped: 0,
      meters_dbfs: [],
      writer_failed: false,
    });
    const s = await ipc.audio.recordStatus();
    expect(s.writer_failed).toBe(false);
    expect(invokeMock).toHaveBeenCalledWith("audio_record_status", undefined);
  });

  it("passes deviceName/channels through to audio_record_start", async () => {
    invokeMock.mockResolvedValue({
      recording: true,
      captured_frames: 0,
      duration_ms: 0,
      dropped: 0,
      meters_dbfs: [],
      writer_failed: false,
    });
    await ipc.audio.recordStart("Scarlett 2i2", 2);
    expect(invokeMock).toHaveBeenCalledWith("audio_record_start", {
      deviceName: "Scarlett 2i2",
      channels: 2,
    });
  });

  it("forwards positionMs to the transport seek command", async () => {
    invokeMock.mockResolvedValue(undefined);
    await ipc.transport.seek(1500);
    expect(invokeMock).toHaveBeenCalledWith("audio_seek", {
      positionMs: 1500,
    });
  });
});

describe("toIPCError", () => {
  it("maps a serialised AppError to an IPCError preserving `code`", () => {
    const err = toIPCError({ code: "audio", message: "no device" });
    expect(err).toBeInstanceOf(IPCError);
    expect(err).toMatchObject({
      name: "IPCError",
      code: "audio",
      message: "no device",
    });
  });

  it("passes through a real Error unchanged", () => {
    const original = new Error("boom");
    expect(toIPCError(original)).toBe(original);
  });

  it("wraps an unknown value as a generic Error", () => {
    const err = toIPCError("weird");
    expect(err).toBeInstanceOf(Error);
    expect(err.message).toBe("weird");
  });
});
