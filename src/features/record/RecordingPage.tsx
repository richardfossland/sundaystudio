import { useState } from "react";
import {
  AlertTriangle,
  ArrowLeft,
  Bookmark,
  CheckCircle2,
  Download,
  Info,
  Loader2,
  Plus,
} from "lucide-react";

import { Button } from "@/components/ui/Button";
import { RecordButton, Timecode, TrackHeader } from "@/components/audio";
import type { TrackState } from "@/components/audio";
import { ipc } from "@/lib/ipc";
import { useSession } from "@/lib/session";
import type { ExportResult, Track } from "@/lib/bindings";

const TRACK_COLORS = [
  "#D4A73A",
  "#3A8DD4",
  "#4CB97A",
  "#D47A3A",
  "#9B6BD4",
  "#D44A6B",
];

/**
 * The recording page (Phase 2.2/2.3 shell): transport + the project's track
 * strips, bound to the open project. Track state (arm/mute/solo/gain) persists
 * through the project store; add tracks and chapter markers here too.
 *
 * The transport's record button is a visual placeholder: live multi-track
 * capture is wired through the recorder engine's cpal stream, which needs real
 * hardware to exercise (Phase 1.2 `stream` + integration in a hardware session).
 * Meters therefore read silence here.
 */
export function RecordingPage({ onBack }: { onBack?: () => void }) {
  const snapshot = useSession((s) => s.snapshot);
  const setSnapshot = useSession((s) => s.setSnapshot);
  const [recordState, setRecordState] = useState<
    "idle" | "armed" | "recording"
  >("idle");
  const [busy, setBusy] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<ExportResult | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  if (!snapshot) return null;
  const { project, tracks, markers } = snapshot;

  async function doExport() {
    setExporting(true);
    setExportError(null);
    setExportResult(null);
    try {
      // WAV is the natively-writable format today; mastered + normalised.
      setExportResult(await ipc.exporter.render("wav-archival"));
    } catch (err) {
      setExportError(err instanceof Error ? err.message : String(err));
    } finally {
      setExporting(false);
    }
  }

  async function refresh() {
    setSnapshot(await ipc.project.snapshot());
  }

  async function withBusy(fn: () => Promise<void>) {
    setBusy(true);
    try {
      await fn();
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
          <Timecode ms={0} size="md" />
          <div className="flex flex-col items-center gap-1">
            <RecordButton
              state={recordState}
              size={52}
              onClick={() =>
                setRecordState((s) =>
                  s === "recording"
                    ? "idle"
                    : s === "armed" || armedCount > 0
                      ? "recording"
                      : "armed",
                )
              }
            />
          </div>
        </div>
      </header>

      {/* Placeholder notice */}
      <div className="flex items-center gap-2 bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs text-[var(--color-fg-muted)]">
        <Info size={13} className="shrink-0" />
        Transport is a preview — live capture wires through the recorder engine
        on real audio hardware. Track settings and chapters below are saved to
        the project.
      </div>

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
        <div className="flex items-start gap-2 border-b border-[var(--color-border)] bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs text-[var(--color-danger)]">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span className="break-all">{exportError}</span>
        </div>
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
            {tracks.map((t) => (
              <TrackHeader
                key={t.id}
                track={toTrackState(t)}
                onChange={(patch) => saveTrack(t, patch)}
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

function toTrackState(t: Track): TrackState {
  return {
    name: t.name,
    color: t.color,
    armed: t.armed,
    muted: t.mute,
    soloed: t.solo,
    monitoring: false,
    gainDb: t.gain_db,
    // No live engine in this shell — meters read silence.
    levelDb: -60,
    peakDb: -60,
  };
}
