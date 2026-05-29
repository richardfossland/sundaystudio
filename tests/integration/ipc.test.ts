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
