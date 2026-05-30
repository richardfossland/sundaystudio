import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ArrowLeft,
  FilePlus2,
  Loader2,
  Magnet,
  Mic,
  Trash2,
  ZoomIn,
  ZoomOut,
} from "lucide-react";

import { Button } from "@/components/ui/Button";
import { Timecode } from "@/components/audio";
import { cn } from "@/lib/cn";
import { ipc } from "@/lib/ipc";
import { useSession } from "@/lib/session";
import { formatTimecode } from "@/lib/timeline";
import type { Region } from "@/lib/bindings";

import { useEditor } from "./editorStore";
import { LANE_H, RULER_H, Timeline } from "./Timeline";

/**
 * The waveform timeline editor (Phase 3.1). Loads the open project's regions,
 * draws each take's waveform on a per-track lane, and lets you scrub, zoom, move
 * and trim clips. Regions are the source of truth in the backend; edits persist
 * immediately and non-destructively (the take WAVs are never rewritten).
 *
 * Until live capture is wired on hardware, "Import audio" lays existing WAVs onto
 * the timeline so the editor is fully usable today.
 */
export function EditPage({
  onBack,
  onOpenRecord,
}: {
  onBack?: () => void;
  onOpenRecord?: () => void;
}) {
  const snapshot = useSession((s) => s.snapshot);
  const setSnapshot = useSession((s) => s.setSnapshot);
  const {
    pxPerSec,
    playheadMs,
    selectedRegionId,
    snapEnabled,
    zoomIn,
    zoomOut,
    setPlayhead,
    select,
    toggleSnap,
  } = useEditor();

  const [regions, setRegions] = useState<Region[]>([]);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Load the timeline once; edits mutate this local copy and persist in the
  // background, so a refetch never clobbers an in-flight drag.
  useEffect(() => {
    let alive = true;
    ipc.edit
      .timeline()
      .then((tl) => alive && setRegions(tl.regions))
      .catch(
        (e) => alive && setError(e instanceof Error ? e.message : String(e)),
      )
      .finally(() => alive && setLoading(false));
    return () => {
      alive = false;
    };
  }, []);

  const commitRegion = useCallback((next: Region) => {
    setRegions((rs) => rs.map((r) => (r.id === next.id ? next : r)));
    ipc.edit
      .updateRegion(next)
      .catch((e) => setError(e instanceof Error ? e.message : String(e)));
  }, []);

  const deleteRegion = useCallback(
    (id: string) => {
      setRegions((rs) => rs.filter((r) => r.id !== id));
      select(null);
      ipc.edit
        .deleteRegion(id)
        .catch((e) => setError(e instanceof Error ? e.message : String(e)));
    },
    [select],
  );

  // Delete/Backspace removes the selected clip (unless typing in a field).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null;
      if (el && /^(INPUT|TEXTAREA)$/.test(el.tagName)) return;
      if ((e.key === "Delete" || e.key === "Backspace") && selectedRegionId) {
        e.preventDefault();
        deleteRegion(selectedRegionId);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [selectedRegionId, deleteRegion]);

  async function importAudio() {
    setError(null);
    let picked: string[];
    try {
      const result = await open({
        multiple: true,
        filters: [{ name: "Audio", extensions: ["wav"] }],
      });
      if (!result) return;
      picked = Array.isArray(result) ? result : [result];
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      return;
    }
    if (picked.length === 0) return;

    setImporting(true);
    try {
      const tl = await ipc.edit.importTakes(picked);
      setRegions(tl.regions);
      // Import may have created tracks — refresh the project snapshot.
      setSnapshot(await ipc.project.snapshot());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setImporting(false);
    }
  }

  if (!snapshot) return null;
  const { project, tracks, markers } = snapshot;
  const hasContent = tracks.length > 0 && regions.length > 0;

  return (
    <div className="flex h-screen flex-col">
      {/* Toolbar */}
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
              Editor · {tracks.length} tracks · {regions.length} clips
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2">
          {onOpenRecord && (
            <Button variant="ghost" size="sm" onClick={onOpenRecord}>
              <Mic size={15} />
              Record
            </Button>
          )}
          <Button
            variant="surface"
            size="sm"
            onClick={importAudio}
            disabled={importing}
          >
            {importing ? (
              <Loader2 size={15} className="animate-spin" />
            ) : (
              <FilePlus2 size={15} />
            )}
            {importing ? "Importing…" : "Import audio"}
          </Button>
          <div className="mx-1 h-5 w-px bg-[var(--color-border)]" />
          <Button
            variant={snapEnabled ? "accent" : "ghost"}
            size="sm"
            onClick={toggleSnap}
            aria-pressed={snapEnabled}
            title="Snap to edges, markers and playhead (S)"
          >
            <Magnet size={15} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={zoomOut}
            aria-label="Zoom out"
          >
            <ZoomOut size={15} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={zoomIn}
            aria-label="Zoom in"
          >
            <ZoomIn size={15} />
          </Button>
          <div className="ml-1 flex flex-col items-end">
            <Timecode ms={playheadMs} size="md" />
            <span className="font-mono text-[10px] text-[var(--color-fg-muted)]">
              {formatTimecode(playheadMs)}
            </span>
          </div>
        </div>
      </header>

      {error && (
        <div className="flex items-start gap-2 border-b border-[var(--color-border)] bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs text-[var(--color-danger)]">
          <AlertTriangle size={14} className="mt-0.5 shrink-0" />
          <span className="break-all">{error}</span>
        </div>
      )}

      {/* Body */}
      {loading ? (
        <div className="grid flex-1 place-items-center text-[var(--color-fg-muted)]">
          <Loader2 size={20} className="animate-spin" />
        </div>
      ) : !hasContent ? (
        <EmptyState onImport={importAudio} importing={importing} />
      ) : (
        <div className="flex flex-1 overflow-hidden">
          {/* Track header column */}
          <div className="w-40 shrink-0 border-r border-[var(--color-border)]">
            <div
              className="border-b border-[var(--color-border)]"
              style={{ height: RULER_H }}
            />
            {tracks.map((t) => (
              <div
                key={t.id}
                className="flex items-center gap-2 border-b border-[var(--color-border)]/60 px-3"
                style={{ height: LANE_H }}
              >
                <span
                  className="size-2.5 shrink-0 rounded-full"
                  style={{ background: t.color }}
                />
                <span className="truncate text-ui-sm">{t.name}</span>
              </div>
            ))}
          </div>

          {/* Scrollable timeline */}
          <Timeline
            tracks={tracks}
            regions={regions}
            markers={markers}
            pxPerSec={pxPerSec}
            playheadMs={playheadMs}
            selectedRegionId={selectedRegionId}
            snapEnabled={snapEnabled}
            onSeek={setPlayhead}
            onSelect={select}
            onCommitRegion={commitRegion}
          />
        </div>
      )}

      {/* Selection action bar */}
      {selectedRegionId && (
        <div className="flex items-center justify-between border-t border-[var(--color-border)] bg-[var(--color-bg-elevated)] px-5 py-2 text-ui-xs">
          <span className="text-[var(--color-fg-muted)]">
            Clip selected — drag to move, drag an edge to trim
          </span>
          <Button
            variant="ghost"
            size="sm"
            onClick={() => deleteRegion(selectedRegionId)}
          >
            <Trash2 size={14} />
            Delete clip
          </Button>
        </div>
      )}
    </div>
  );
}

function EmptyState({
  onImport,
  importing,
}: {
  onImport: () => void;
  importing: boolean;
}) {
  return (
    <div className={cn("grid flex-1 place-items-center px-6 text-center")}>
      <div className="max-w-sm">
        <FilePlus2
          size={28}
          className="mx-auto mb-3 text-[var(--color-fg-muted)]"
        />
        <h2 className="text-ui-md font-semibold">
          Nothing on the timeline yet
        </h2>
        <p className="mb-4 mt-1 text-ui-sm text-[var(--color-fg-muted)]">
          Import existing WAV recordings to lay them onto tracks and start
          editing. Each file becomes a clip you can move, trim and arrange.
        </p>
        <Button variant="accent" onClick={onImport} disabled={importing}>
          {importing ? (
            <Loader2 size={16} className="animate-spin" />
          ) : (
            <FilePlus2 size={16} />
          )}
          {importing ? "Importing…" : "Import audio"}
        </Button>
      </div>
    </div>
  );
}
