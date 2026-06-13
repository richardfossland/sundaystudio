// Integration — the recording page's transport is wired to the engine commands.
// `@/lib/ipc` is fully mocked so the flow runs offline (no audio hardware): the
// record button drives `audio_record_start` / `audio_record_stop`, the live
// `audio_record_status` poll feeds the timecode + per-track meters, and a failed
// start surfaces the action-error banner. The recording page also reads the open
// project from the session store and polls status via TanStack Query, so the
// render is wrapped in a QueryClientProvider and the session is seeded first.
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import {
  render,
  screen,
  fireEvent,
  waitFor,
  cleanup,
} from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { createElement } from "react";

import type {
  AudioSettings,
  Project,
  ProjectSnapshot,
  RecordingStatus,
  TimelineSnapshot,
  Track,
} from "@/lib/bindings";

const {
  getSettingsMock,
  recordStartMock,
  recordStopMock,
  recordStatusMock,
  snapshotMock,
  exportRenderMock,
  exportPresetsMock,
} = vi.hoisted(() => ({
  getSettingsMock: vi.fn(),
  recordStartMock: vi.fn(),
  recordStopMock: vi.fn(),
  recordStatusMock: vi.fn(),
  snapshotMock: vi.fn(),
  exportRenderMock: vi.fn(),
  exportPresetsMock: vi.fn(),
}));

vi.mock("@/lib/ipc", async () => {
  const actual = await vi.importActual<typeof import("@/lib/ipc")>("@/lib/ipc");
  return {
    // Keep the real `errorMessage` (used by the page's catch handlers).
    errorMessage: actual.errorMessage,
    ipc: {
      audio: {
        getSettings: getSettingsMock,
        recordStart: recordStartMock,
        recordStop: recordStopMock,
        recordStatus: recordStatusMock,
      },
      project: {
        snapshot: snapshotMock,
        backup: vi.fn(),
        addTrack: vi.fn(),
        updateTrack: vi.fn(),
        addMarker: vi.fn(),
      },
      exporter: { render: exportRenderMock, presets: exportPresetsMock },
    },
  };
});

import { RecordingPage } from "@/features/record/RecordingPage";
import { useSession } from "@/lib/session";
import { useI18n } from "@/lib/i18n";

function track(overrides: Partial<Track> = {}): Track {
  return {
    id: "t1",
    project_id: "p1",
    name: "Track 1",
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
    ...overrides,
  } as Track;
}

function project(): Project {
  return {
    id: "p1",
    name: "Sunday Pod",
    sample_rate: 48000,
    channel_count: 2,
    created_at: 0,
  } as Project;
}

function snapshot(tracks: Track[]): ProjectSnapshot {
  return { project: project(), tracks, markers: [] } as ProjectSnapshot;
}

function settings(): AudioSettings {
  return {
    input_device: "Focusrite Scarlett",
    output_device: null,
    sample_rate: 48000,
    buffer_size: 256,
  };
}

function status(overrides: Partial<RecordingStatus> = {}): RecordingStatus {
  return {
    recording: true,
    captured_frames: 0,
    duration_ms: 0,
    dropped: 0,
    meters_dbfs: [],
    writer_failed: false,
    ...overrides,
  };
}

const emptyTimeline: TimelineSnapshot = {
  takes: [],
  regions: [],
} as TimelineSnapshot;

function renderPage() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return render(
    createElement(QueryClientProvider, { client }, <RecordingPage />),
  );
}

describe("RecordingPage transport wiring", () => {
  beforeEach(() => {
    getSettingsMock.mockReset().mockResolvedValue(settings());
    recordStartMock.mockReset().mockResolvedValue(status({ recording: true }));
    recordStopMock.mockReset().mockResolvedValue(emptyTimeline);
    // While rolling, the poll keeps recording=true so the sync effect doesn't
    // bounce the button back to idle.
    recordStatusMock.mockReset().mockResolvedValue(status({ recording: true }));
    snapshotMock.mockReset().mockResolvedValue(snapshot([track()]));
    exportRenderMock.mockReset();
    exportPresetsMock.mockReset().mockResolvedValue([
      { id: "general-podcast", label: "General podcast host" },
      { id: "spotify", label: "Spotify for Podcasters" },
      { id: "wav-archival", label: "WAV (archival)" },
    ]);
    useI18n.getState().setLang("en");
    // Two tracks; the first armed so a single press starts a take.
    useSession
      .getState()
      .setSnapshot(
        snapshot([
          track({ id: "t1", name: "Host", armed: true }),
          track({ id: "t2", name: "Guest", armed: false }),
        ]),
      );
  });

  afterEach(() => {
    cleanup();
    useSession.getState().close();
  });

  it("starts a take via audio_record_start with the configured device + a channel per track", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));

    await waitFor(() => expect(recordStartMock).toHaveBeenCalledOnce());
    expect(getSettingsMock).toHaveBeenCalledOnce();
    // (deviceName from settings, one channel per project track)
    expect(recordStartMock).toHaveBeenCalledWith("Focusrite Scarlett", 2);
    // The button now offers to stop.
    await screen.findByRole("button", { name: /stop recording/i });
  });

  it("stops the take via audio_record_stop and refreshes the timeline", async () => {
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    const stop = await screen.findByRole("button", {
      name: /stop recording/i,
    });

    fireEvent.click(stop);
    await waitFor(() => expect(recordStopMock).toHaveBeenCalledOnce());
    // Refresh pulls the updated snapshot so the new take's regions appear.
    expect(snapshotMock).toHaveBeenCalled();
  });

  it("arms first when nothing is armed, then starts on the next press", async () => {
    useSession
      .getState()
      .setSnapshot(snapshot([track({ id: "t1", armed: false })]));
    renderPage();

    // First press only arms — no engine call yet.
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    expect(recordStartMock).not.toHaveBeenCalled();

    // Second press starts the take.
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    await waitFor(() => expect(recordStartMock).toHaveBeenCalledOnce());
  });

  it("surfaces a start failure in the action-error banner without claiming to record", async () => {
    recordStartMock.mockRejectedValue(new Error("no input device"));
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));

    await screen.findByTestId("action-error");
    expect(screen.getByTestId("action-error")).toHaveTextContent(
      /no input device/i,
    );
    // Still idle — the button did not flip to "stop".
    expect(
      screen.queryByRole("button", { name: /stop recording/i }),
    ).not.toBeInTheDocument();
  });

  it("exports the chosen format preset (not just WAV)", async () => {
    renderPage();
    // The picker is populated from the presets query.
    const picker = (await screen.findByLabelText(
      "Export format",
    )) as HTMLSelectElement;
    fireEvent.change(picker, { target: { value: "spotify" } });
    fireEvent.click(screen.getByRole("button", { name: /^export$/i }));

    await waitFor(() =>
      // No editor-curated chapters in this flow, so chapters ride along as
      // `undefined` (the master-preset arg is also left to the backend default).
      expect(exportRenderMock).toHaveBeenCalledWith(
        "spotify",
        undefined,
        undefined,
      ),
    );
  });

  it("embeds the editor's show-notes chapters on export when present", async () => {
    // The show-notes panel lifts curated chapters into the session; the export
    // call must carry them so ffmpeg can write chapter metadata on the file.
    useSession.getState().setChapters([
      { start_ms: 0, title: "Welcome" },
      { start_ms: 120000, title: "Interview" },
    ]);
    renderPage();
    const picker = (await screen.findByLabelText(
      "Export format",
    )) as HTMLSelectElement;
    fireEvent.change(picker, { target: { value: "spotify" } });
    fireEvent.click(screen.getByRole("button", { name: /^export$/i }));

    await waitFor(() =>
      expect(exportRenderMock).toHaveBeenCalledWith("spotify", undefined, [
        { start_ms: 0, title: "Welcome" },
        { start_ms: 120000, title: "Interview" },
      ]),
    );
  });

  it("feeds the live status into the timecode and per-track meters", async () => {
    recordStatusMock.mockResolvedValue(
      status({ recording: true, duration_ms: 5000, meters_dbfs: [-12, -24] }),
    );
    renderPage();
    fireEvent.click(screen.getByRole("button", { name: /^record$/i }));
    await screen.findByRole("button", { name: /stop recording/i });

    // Timecode reflects the captured duration…
    await screen.findByText("00:00:05.000");
    // …and each track's readout reflects its channel's peak.
    await screen.findByText("-12.0 dB");
    expect(screen.getByText("-24.0 dB")).toBeInTheDocument();
  });
});
