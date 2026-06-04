// Integration smoke — the IPC client layer. Mocks Tauri's `invoke` for the
// happy path, and tests the AppError -> IPCError mapping via the pure
// `toIPCError` helper (no async rejection plumbing required).
import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: invokeMock }));

import { ipc, toIPCError, IPCError, errorMessage } from "@/lib/ipc";

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

// ── Arg-name contract ────────────────────────────────────────────────────────
//
// The IPC boundary is dynamically typed: Tauri matches the JS arg-object keys to
// the Rust command parameters by name (camelCase JS ↔ snake_case Rust, applied
// automatically by Tauri). A hand-written wrapper that passes the wrong KEY —
// e.g. `{ device }` instead of `{ deviceName }`, or a snake_case key where Tauri
// expects camelCase — compiles green but the argument arrives as `undefined` at
// runtime and the feature silently breaks. TypeScript cannot catch this class.
//
// These tests pin the exact `invoke(name, args)` shape every payload-passing
// wrapper emits, so any future edit that changes a key (or the command name)
// fails here instead of on the rig. Each expected key is the camelCase form of
// the Rust parameter (e.g. Rust `device_name` ⇒ JS `deviceName`).
describe("ipc arg-name contract", () => {
  beforeEach(() => invokeMock.mockReset());

  // Cases: [label, () => call, expected command name, expected args object].
  // `null` args means the wrapper must invoke with no argument object.
  const region = {
    id: "r1",
    take_id: "t1",
    source_track_id: "s1",
    target_track_id: "tt1",
    start_in_take_ms: 0,
    end_in_take_ms: 1000,
    position_in_timeline_ms: 0,
    fade_in_ms: 5,
    fade_out_ms: 5,
    gain_adjust_db: 0,
  };
  const track = {
    id: "tk1",
    project_id: "p1",
    name: "Mic 1",
    color: "#D4A73A",
    input_assignment: null,
    output_assignment: null,
    gain_db: 0,
    pan: 0,
    mute: false,
    solo: false,
    armed: false,
    position: 0,
    voice_preset: null,
  };
  const settings = {
    input_device: null,
    output_device: null,
    sample_rate: 48000,
    buffer_size: 256,
  };
  const jingleSpec = {
    title: "Intro",
    duration_sec: 30 as const,
    mood: "energetic" as const,
    tempo_bpm: 120,
    instruments: ["piano"],
  };

  const cases: Array<[string, () => Promise<unknown>, string, unknown]> = [
    // App / no-arg pollers
    ["app.info", () => ipc.app.info(), "app_info", undefined],
    ["audio.devices", () => ipc.audio.devices(), "audio_devices", undefined],
    [
      "audio.recordTestTone",
      () => ipc.audio.recordTestTone(),
      "audio_record_test_tone",
      undefined,
    ],
    [
      "audio.getSettings",
      () => ipc.audio.getSettings(),
      "audio_get_settings",
      undefined,
    ],
    [
      "audio.recordStop",
      () => ipc.audio.recordStop(),
      "audio_record_stop",
      undefined,
    ],
    [
      "audio.recordStatus",
      () => ipc.audio.recordStatus(),
      "audio_record_status",
      undefined,
    ],

    // Audio settings / estimate — snake_case Rust params (new_settings,
    // sample_rate, buffer_size) ⇒ camelCase JS keys.
    [
      "audio.setSettings",
      () => ipc.audio.setSettings(settings),
      "audio_set_settings",
      { newSettings: settings },
    ],
    [
      "audio.latencyEstimate",
      () => ipc.audio.latencyEstimate(48000, 256),
      "audio_latency_estimate",
      { sampleRate: 48000, bufferSize: 256 },
    ],
    [
      "audio.recordStart (defaults)",
      () => ipc.audio.recordStart(),
      "audio_record_start",
      { deviceName: undefined, channels: undefined },
    ],

    // Transport
    [
      "transport.play",
      () => ipc.transport.play(),
      "audio_play_timeline",
      undefined,
    ],
    ["transport.resume", () => ipc.transport.resume(), "audio_play", undefined],
    ["transport.pause", () => ipc.transport.pause(), "audio_pause", undefined],
    [
      "transport.muteTrack",
      () => ipc.transport.muteTrack(2, true),
      "audio_playback_mute",
      { trackIdx: 2, muted: true },
    ],
    [
      "transport.status",
      () => ipc.transport.status(),
      "audio_playback_status",
      undefined,
    ],
    [
      "transport.stop",
      () => ipc.transport.stop(),
      "audio_stop_playback",
      undefined,
    ],

    // Project create / open / registry — these carry the most keys, so a typo
    // here is the highest-value thing to pin.
    [
      "project.create",
      () => ipc.project.create("/tmp/p.scast", "Pod", 48000, 2),
      "project_create",
      {
        path: "/tmp/p.scast",
        name: "Pod",
        sampleRate: 48000,
        channelCount: 2,
      },
    ],
    [
      "project.createFromTemplate",
      () => ipc.project.createFromTemplate("/tmp/p.scast", "Pod", "interview"),
      "project_create_from_template",
      { path: "/tmp/p.scast", name: "Pod", templateId: "interview" },
    ],
    [
      "project.open",
      () => ipc.project.open("/tmp/p.scast"),
      "project_open",
      { path: "/tmp/p.scast" },
    ],
    [
      "project.rename",
      () => ipc.project.rename("New name"),
      "project_rename",
      { name: "New name" },
    ],
    [
      "project.addTrack",
      () => ipc.project.addTrack("Mic 1", "#D4A73A"),
      "track_add",
      { name: "Mic 1", color: "#D4A73A" },
    ],
    [
      "project.updateTrack",
      () => ipc.project.updateTrack(track),
      "track_update",
      { track },
    ],
    [
      "project.deleteTrack",
      () => ipc.project.deleteTrack("tk1"),
      "track_delete",
      { id: "tk1" },
    ],
    [
      "project.addMarker",
      () => ipc.project.addMarker(1500, "Chapter 1", "#D4A73A"),
      "marker_add",
      { positionMs: 1500, label: "Chapter 1", color: "#D4A73A" },
    ],
    [
      "project.deleteMarker",
      () => ipc.project.deleteMarker("m1"),
      "marker_delete",
      { id: "m1" },
    ],
    [
      "project.new",
      () => ipc.project.new("Pod"),
      "project_new",
      { name: "Pod" },
    ],
    [
      "project.save",
      () => ipc.project.save("reg1", "Pod"),
      "project_save",
      { registryId: "reg1", name: "Pod" },
    ],
    [
      "project.load",
      () => ipc.project.load("reg1"),
      "project_load",
      { registryId: "reg1" },
    ],
    [
      "project.delete",
      () => ipc.project.delete("reg1"),
      "project_delete",
      { registryId: "reg1" },
    ],

    // DSP
    [
      "dsp.analyzeFile",
      () => ipc.dsp.analyzeFile("/tmp/a.wav"),
      "dsp_analyze_file",
      { path: "/tmp/a.wav" },
    ],

    // Edit / timeline — the region/silence ops carry many ms keys; pin them all.
    [
      "edit.peaks",
      () => ipc.edit.peaks("t1", "s1"),
      "audio_peaks",
      { takeId: "t1", sourceTrackId: "s1" },
    ],
    [
      "edit.analyzeSilence",
      () => ipc.edit.analyzeSilence("t1", "s1", -50, 500),
      "analyze_silence",
      {
        takeId: "t1",
        sourceTrackId: "s1",
        thresholdDb: -50,
        minSilenceMs: 500,
      },
    ],
    [
      "edit.addRegion",
      () => ipc.edit.addRegion("t1", "s1", "tt1", 0, 1000, 250),
      "region_add",
      {
        takeId: "t1",
        sourceTrackId: "s1",
        targetTrackId: "tt1",
        startInTakeMs: 0,
        endInTakeMs: 1000,
        positionInTimelineMs: 250,
      },
    ],
    [
      "edit.createRegion",
      () => ipc.edit.createRegion(region),
      "region_create",
      { region },
    ],
    [
      "edit.updateRegion",
      () => ipc.edit.updateRegion(region),
      "region_update",
      { region },
    ],
    [
      "edit.deleteRegion",
      () => ipc.edit.deleteRegion("r1"),
      "region_delete",
      { id: "r1" },
    ],
    [
      "edit.importTakes",
      () => ipc.edit.importTakes(["/a.wav", "/b.wav"]),
      "take_import",
      { paths: ["/a.wav", "/b.wav"] },
    ],
    ["edit.timeline", () => ipc.edit.timeline(), "project_timeline", undefined],

    // Export
    [
      "exporter.render (preset only)",
      () => ipc.exporter.render("wav-archival"),
      "export_render",
      { presetId: "wav-archival", masterPresetId: undefined },
    ],
    [
      "exporter.render (with master)",
      () => ipc.exporter.render("wav-archival", "conversation-podcast"),
      "export_render",
      { presetId: "wav-archival", masterPresetId: "conversation-podcast" },
    ],

    // AI
    [
      "ai.generateJingle",
      () => ipc.ai.generateJingle(jingleSpec),
      "ai_jingle_generate",
      { spec: jingleSpec },
    ],

    // Deep link (Rec → Studio import handoff)
    [
      "deeplink.parseImport",
      () => ipc.deeplink.parseImport("sundaystudio://import?path=/a.wav"),
      "deeplink_parse_import",
      { url: "sundaystudio://import?path=/a.wav" },
    ],
  ];

  it.each(cases)(
    "%s invokes with the pinned command + arg shape",
    async (_label, callWrapper, cmd, args) => {
      invokeMock.mockResolvedValue(undefined);
      await callWrapper();
      expect(invokeMock).toHaveBeenCalledWith(cmd, args);
    },
  );
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

describe("errorMessage", () => {
  it("reads .message from an Error", () => {
    expect(errorMessage(new Error("boom"))).toBe("boom");
  });

  it("reads .message from an IPCError (the typed Tauri error)", () => {
    const e = new IPCError({ code: "io", message: "disk full" });
    expect(errorMessage(e)).toBe("disk full");
  });

  it("reads .message from a bare {message} object (raw Tauri reject)", () => {
    expect(errorMessage({ code: "validation", message: "bad spec" })).toBe(
      "bad spec",
    );
  });

  it("falls back to String() for primitives", () => {
    expect(errorMessage("weird")).toBe("weird");
    expect(errorMessage(42)).toBe("42");
    expect(errorMessage(null)).toBe("null");
  });

  it("ignores a non-string message field", () => {
    expect(errorMessage({ message: 123 })).toBe("[object Object]");
  });
});
