import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowLeft,
  Bookmark,
  CheckCircle2,
  Download,
  HardDriveDownload,
  Info,
  Loader2,
  Plus,
  SlidersHorizontal,
} from "lucide-react";

import { Button } from "@/components/ui/Button";
import { StatusBanner } from "@/components/ui/StatusBanner";
import { RecordButton, Timecode, TrackHeader } from "@/components/audio";
import type { TrackState } from "@/components/audio";
import { errorMessage, ipc } from "@/lib/ipc";
import { useSession } from "@/lib/session";
import { t } from "@/lib/i18n";
import { useRecordingStatus } from "@/lib/useRecordingStatus";
import type { ExportPresetInfo, ExportResult, Track } from "@/lib/bindings";

const TRACK_COLORS = [
  "#D4A73A",
  "#3A8DD4",
  "#4CB97A",
  "#D47A3A",
  "#9B6BD4",
  "#D44A6B",
];

/**
 * The recording page (Phase 2.2/2.3): transport + the project's track strips,
 * bound to the open project. Track state (arm/mute/solo/gain) persists through
 * the project store; add tracks and chapter markers here too.
 *
 * The transport now drives the real recorder engine: the record button calls
 * `audio_record_start`/`audio_record_stop`, the timecode and per-track meters
 * read the live `audio_record_status` poll, and stop lays the captured WAVs onto
 * the timeline. The cpal input stream itself is HARDWARE-UNVERIFIED — it needs a
 * real audio interface to exercise — but the UI↔engine wiring is live: with no
 * device the start call surfaces the engine's error, and meters read silence
 * (−60 dB) until a take is rolling.
 */
export function RecordingPage({
  onBack,
  onOpenEdit,
}: {
  onBack?: () => void;
  onOpenEdit?: () => void;
}) {
  const snapshot = useSession((s) => s.snapshot);
  const setSnapshot = useSession((s) => s.setSnapshot);
  const [recordState, setRecordState] = useState<
    "idle" | "armed" | "recording"
  >("idle");
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);
  // Default to a ready-to-publish MP3; the WAV master is always available too.
  // (If ffmpeg is somehow missing, the backend keeps the WAV and says so in note.)
  const [exportPreset, setExportPreset] = useState("general-podcast");
  const [backingUp, setBackingUp] = useState(false);
  const [backupNote, setBackupNote] = useState<{
    kind: "ok" | "error";
    text: string;
  } | null>(null);

  // The platform-ready export presets for the format picker (format + bitrate +
  // LUFS target). Loaded once; falls back to just WAV if the call ever fails.
  const { data: presets } = useQuery<ExportPresetInfo[]>({
    queryKey: ["export_presets"],
    queryFn: ipc.exporter.presets,
  });

  // Poll the live transport: the raw status drives the timecode + per-track
  // meters, and the derived alerts surface the safety banners (writer-failed /
  // dropped). Polling stays on whenever a take is armed or rolling; idle status
  // is cheap and returns the idle snapshot.
  const { status, alerts } = useRecordingStatus({
    enabled: recordState !== "idle",
  });

  // Keep the UI in sync if the engine stops on its own — a writer/disk failure
  // or a lost device ends the take backend-side, and the button must fall back
  // to idle rather than claim it's still rolling.
  useEffect(() => {
    if (recordState === "recording" && status && !status.recording) {
      setRecordState("idle");
    }
  }, [recordState, status]);

  if (!snapshot) return null;
  const { project, tracks, markers } = snapshot;

  async function doExport() {
    setExporting(true);
    setExportError(null);
    setExportResult(null);
    try {
      // Mastered + loudness-normalised, then encoded to the chosen preset's
      // format (MP3/AAC via the bundled ffmpeg; WAV is native).
      setExportResult(await ipc.exporter.render(exportPreset));
    } catch (err) {
      setExportError(errorMessage(err));
    } finally {
      setExporting(false);
    }
  }

  async function doBackup() {
    setBackingUp(true);
    setBackupNote(null);
    try {
      await ipc.project.backup();
      setBackupNote({ kind: "ok", text: t("recordBackupDone") });
    } catch (err) {
      const detail = errorMessage(err);
      setBackupNote({
        kind: "error",
        text: `${t("recordBackupFailed")}: ${detail}`,
      });
    } finally {
      setBackingUp(false);
      // Auto-dismiss the confirmation toast after a few seconds.
      window.setTimeout(() => setBackupNote(null), 4000);
    }
  }

  async function refresh() {
    setSnapshot(await ipc.project.snapshot());
  }

  // The record button's three-state flow, now wired to the engine:
  //   idle → (tracks armed?) → armed → recording → idle
  // Start arms one capture channel per project track (the engine maps channel i
  // → track i, exactly how stop lays the WAVs onto the timeline) and honours the
  // input device chosen in Settings. Stop finalises the take and refreshes the
  // timeline. Failures surface through the same banner as every other mutation.
  async function toggleRecord() {
    if (recordState === "recording") {
      setBusy(true);
      setActionError(null);
      try {
        await ipc.audio.recordStop();
        await refresh();
        setRecordState("idle");
      } catch (err) {
        setActionError(`${t("recordActionFailed")}: ${errorMessage(err)}`);
      } finally {
        setBusy(false);
      }
      return;
    }
    // Not rolling: first press arms (when nothing is armed yet), the next starts.
    if (recordState !== "armed" && armedCount === 0) {
      setRecordState("armed");
      return;
    }
    setBusy(true);
    setActionError(null);
    try {
      const settings = await ipc.audio.getSettings();
      await ipc.audio.recordStart(
        settings.input_device ?? undefined,
        tracks.length,
      );
      setRecordState("recording");
    } catch (err) {
      setActionError(`${t("recordActionFailed")}: ${errorMessage(err)}`);
      setRecordState("idle");
    } finally {
      setBusy(false);
    }
  }

  // Track / marker mutations all flow through here. Previously a failed invoke
  // threw an unhandled rejection and the user saw nothing change — every other
  // mutation path on this page surfaces its error, so this one must too.
  async function withBusy(fn: () => Promise<void>) {
    setBusy(true);
    setActionError(null);
    try {
      await fn();
    } catch (err) {
      setActionError(`${t("recordActionFailed")}: ${errorMessage(err)}`);
    } finally {
      setBusy(false);
    }
  }

  const saveTrack = (track: Track, patch: Partial<TrackState>) =>
    withBusy(async () => {
      const next: Track = {
        ...track,
        name: patch.name ?? track.name,
        color: patch.color ?? track.color,
        armed: patch.armed ?? track.armed,
        mute: patch.muted ?? track.mute,
        solo: patch.soloed ?? track.solo,
        gain_db: patch.gainDb ?? track.gain_db,
      };
      await ipc.project.updateTrack(next);
      await refresh();
    });

  const addTrack = () =>
    withBusy(async () => {
      const color = TRACK_COLORS[tracks.length % TRACK_COLORS.length];
      await ipc.project.addTrack(`Track ${tracks.length + 1}`, color);
      await refresh();
    });

  const addMarker = () =>
    withBusy(async () => {
      await ipc.project.addMarker(
        0,
        `Chapter ${markers.length + 1}`,
        "#D4A73A",
      );
      await refresh();
    });

  const armedCount = tracks.filter((t) => t.armed).length;

  return (
    <div className="flex h-screen flex-col">
      {/* Top bar */}
      <header className="flex items-center justify-between border-b border-[var(--color-border)] px-5 py-3">
        <div className="flex items-center gap-3">
          {onBack && (
            <Button
              variant="ghost"
              size="sm"
              onClick={onBack}
              aria-label="Back"
            >
              <ArrowLeft size={16} />
            </Button>
          )}
          <div>
            <div className="text-ui-sm font-semibold">{project.name}</div>
            <div className="font-mono text-[11px] text-[var(--color-fg-muted)]">
              {project.sample_rate / 1000} kHz · {project.channel_count} in ·{" "}
              {tracks.length} tracks
            </div>
          </div>
        </div>

        <div className="flex items-center gap-5">
          {onOpenEdit && (
            <Button variant="ghost" size="sm" onClick={onOpenEdit}>
              <SlidersHorizontal size={15} />
              Edit
            </Button>
          )}
          <Button
            variant="surface"
            size="sm"
            onClick={doBackup}
            disabled={backingUp}
          >
            {backingUp ? (
              <Loader2 size={15} className="animate-spin" />
            ) : (
              <HardDriveDownload size={15} />
            )}
            {t("recordBackupProject")}
          </Button>
          {presets && presets.length > 0 && (
            <select
              aria-label="Export format"
              value={exportPreset}
              onChange={(e) => setExportPreset(e.target.value)}
              disabled={exporting}
              className="rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-2 py-1 text-ui-xs"
            >
              {presets.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.label}
                </option>
              ))}
            </select>
          )}
          <Button
            variant="surface"
            size="sm"
            onClick={doExport}
            disabled={exporting}
          >
            {exporting ? (
              <Loader2 size={15} className="animate-spin" />
            ) : (
              <Download size={15} />
            )}
            {exporting ? "Exporting…" : "Export"}
          </Button>
          {alerts.hasDropped && (
            <span
              role="status"
              data-testid="dropped-badge"
              className="flex items-center gap-1 rounded-[var(--radius-sm)] bg-[var(--color-warning)]/15 px-2 py-1 text-ui-xs font-medium text-[var(--color-warning)]"
            >
              <AlertTriangle size={12} className="shrink-0" />
              {t("recordDroppedBadge").replace(
                "{count}",
                String(alerts.dropped),
              )}
            </span>
          )}
          <Timecode ms={status?.duration_ms ?? 0} size="md" />
          <div className="flex flex-col items-center gap-1">
            <RecordButton
              state={recordState}
              size={52}
              disabled={busy}
              onClick={toggleRecord}
            />
          </div>
        </div>
      </header>

      {/* Writer-failure banner — recording is sacred: a disk-write failure
          means the take is being lost, so surface it loudly above everything. */}
      {alerts.writerFailed && (
        <StatusBanner
          kind="critical"
          testId="writer-failed-banner"
          message={t("recordWriterFailed")}
        />
      )}

      {/* A track/marker mutation failed — surface it instead of silently
          dropping the change. */}
      {actionError && (
        <StatusBanner
          kind="danger"
          testId="action-error"
          message={actionError}
        />
      )}

      {/* Capture notice */}
      <div className="flex items-center gap-2 bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs text-[var(--color-fg-muted)]">
        <Info size={13} className="shrink-0" />
        Each take writes continuously to disk, one WAV per track — a crash
        leaves a playable file. Track settings and chapters below are saved to
        the project. Recording needs a connected audio input.
      </div>

      {/* Backup confirmation toast (inline, auto-dismissing) */}
      {backupNote && (
        <StatusBanner
          kind={backupNote.kind === "ok" ? "success" : "danger"}
          testId="backup-note"
          message={backupNote.text}
        />
      )}

      {/* Export result / error */}
      {exportResult && (
        <div className="flex items-start gap-2 border-b border-[var(--color-border)] bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs">
          <CheckCircle2
            size={14}
            className="mt-0.5 shrink-0 text-[var(--color-success)]"
          />
          <div>
            <span className="font-medium">
              Exported{" "}
              {exportResult.loudness.after.integrated_lufs !== null &&
                `at ${exportResult.loudness.after.integrated_lufs.toFixed(1)} LUFS · `}
              {(exportResult.bytes / 1024 / 1024).toFixed(1)} MB
            </span>{" "}
            <code className="break-all text-[var(--color-fg-muted)]">
              {exportResult.output_path}
            </code>
            {exportResult.note && (
              <div className="mt-0.5 text-[var(--color-fg-muted)]">
                {exportResult.note}
              </div>
            )}
          </div>
        </div>
      )}
      {exportError && (
        <StatusBanner
          kind="danger"
          testId="export-error"
          message={exportError}
        />
      )}

      {/* Tracks */}
      <div className="flex-1 overflow-y-auto px-5 py-4">
        {tracks.length === 0 ? (
          <div className="grid h-full place-items-center text-center">
            <div>
              <p className="mb-3 text-ui-sm text-[var(--color-fg-muted)]">
                No tracks yet.
              </p>
              <Button variant="accent" onClick={addTrack} disabled={busy}>
                <Plus size={16} />
                Add track
              </Button>
            </div>
          </div>
        ) : (
          <div className="space-y-2">
            {tracks.map((track, idx) => (
              <TrackHeader
                key={track.id}
                track={toTrackState(track, status?.meters_dbfs?.[idx])}
                onChange={(patch) => saveTrack(track, patch)}
              />
            ))}
            <Button variant="surface" onClick={addTrack} disabled={busy}>
              <Plus size={16} />
              Add track
            </Button>
          </div>
        )}

        {/* Chapters */}
        <section className="mt-8">
          <div className="mb-3 flex items-center justify-between">
            <h2 className="flex items-center gap-1.5 text-ui-xs font-semibold uppercase tracking-wider text-[var(--color-fg-muted)]">
              <Bookmark size={13} />
              Chapters · {markers.length}
            </h2>
            <Button
              variant="ghost"
              size="sm"
              onClick={addMarker}
              disabled={busy}
            >
              <Plus size={14} />
              Add chapter
            </Button>
          </div>
          {markers.length > 0 && (
            <ul className="flex flex-col gap-1.5">
              {markers.map((m) => (
                <li
                  key={m.id}
                  className="flex items-center gap-2 rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-3 py-1.5 text-ui-sm"
                >
                  <span
                    className="size-2 rounded-full"
                    style={{ background: m.color }}
                  />
                  {m.label}
                </li>
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

function toTrackState(track: Track, meterDb?: number | null): TrackState {
  // The engine reports one per-channel peak (dBFS) per poll; channel i maps to
  // track i. serde renders -inf as null, so coalesce null/undefined to silence.
  const level = meterDb ?? -60;
  return {
    name: track.name,
    color: track.color,
    armed: track.armed,
    muted: track.mute,
    soloed: track.solo,
    monitoring: false,
    gainDb: track.gain_db,
    levelDb: level,
    peakDb: level,
  };
}
