import { useCallback, useEffect, useRef, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { open } from "@tauri-apps/plugin-dialog";
import {
  AlertTriangle,
  ArrowLeft,
  Blend,
  ClipboardPaste,
  Combine,
  Copy,
  Eraser,
  FilePlus2,
  FoldHorizontal,
  Loader2,
  Magnet,
  Mic,
  Minus,
  Plus,
  Redo2,
  Scissors,
  Sparkles,
  SquareScissors,
  Trash2,
  Undo2,
  Volume2,
  ZoomIn,
  ZoomOut,
} from "lucide-react";

import { Button } from "@/components/ui/Button";
import { Timecode } from "@/components/audio";
import { cn } from "@/lib/cn";
import {
  applyOps,
  crossfadeOps,
  invertOps,
  mergeableNext,
  mergeOps,
  overlapWithPrev,
  pasteRegion,
  removeSilencesOps,
  rippleDeleteOps,
  splitRegion,
  type EditCommand,
  type PrimOp,
} from "@/lib/editing";
import { ipc } from "@/lib/ipc";
import { PersistQueue } from "@/lib/persistQueue";
import { useSession } from "@/lib/session";
import { formatTimecode } from "@/lib/timeline";
import type { Region, Track } from "@/lib/bindings";

import { useEditor } from "./editorStore";
import { LANE_H, RULER_H, Timeline } from "./Timeline";

/** Per-region gain bounds and step (dB). */
const GAIN_MIN = -24;
const GAIN_MAX = 24;
const GAIN_STEP = 1;
/** Keep the undo history bounded (the plan's cap). */
const HISTORY_CAP = 200;
/** Silence-detection defaults: conservative, so it never cuts into speech. */
const SILENCE_DB = -50;
const MIN_SILENCE_MS = 500;

/**
 * The waveform timeline editor. Loads the open project's regions, draws each
 * take's waveform on a per-track lane, and supports non-destructive editing:
 * scrub, zoom, move/trim/fade clips (Phase 3.1) plus split, ripple-delete, gain
 * and undo/redo (Phase 3.2). Every edit is a command of primitive region ops, so
 * it drives the local timeline and backend identically and reverses cleanly.
 *
 * Regions are the source of truth; edits never rewrite the take WAVs. Until live
 * capture is wired on hardware, "Import audio" lays existing WAVs onto the
 * timeline so the editor is fully usable today.
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
  const [past, setPast] = useState<EditCommand[]>([]);
  const [future, setFuture] = useState<EditCommand[]>([]);
  const [loading, setLoading] = useState(true);
  const [importing, setImporting] = useState(false);
  const [analyzing, setAnalyzing] = useState(false);
  const [leveling, setLeveling] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  // The copied clip lives in a ref (no re-render on copy); a flag drives the UI.
  const clipboardRef = useRef<Region | null>(null);
  const [hasClipboard, setHasClipboard] = useState(false);

  // Refs mirror the latest state so event handlers (drags, keyboard) read
  // current values without stale closures or nested setState.
  const regionsRef = useRef(regions);
  const pastRef = useRef(past);
  const futureRef = useRef(future);
  regionsRef.current = regions;
  pastRef.current = past;
  futureRef.current = future;

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

  // Single FIFO persistence queue for the page's lifetime: ops persist strictly
  // in the order they were applied locally, one at a time. Without this, ops
  // were fired un-awaited and two edits touching the same region id could reach
  // the backend out of order (e.g. update(B) before create(B) → NotFound, which
  // was swallowed while local state already showed the edit → the DB silently
  // diverged from the timeline and the edit vanished on reload).
  const persistQueueRef = useRef<PersistQueue<PrimOp> | null>(null);
  if (!persistQueueRef.current) {
    persistQueueRef.current = new PersistQueue<PrimOp>(
      (op) =>
        op.kind === "create"
          ? ipc.edit.createRegion(op.region)
          : op.kind === "delete"
            ? ipc.edit.deleteRegion(op.region.id)
            : ipc.edit.updateRegion(op.after),
      (_op, e) => setError(e instanceof Error ? e.message : String(e)),
    );
  }

  // Push each primitive op to the backend (create / update / delete), in order.
  const persistOps = useCallback((ops: PrimOp[]) => {
    persistQueueRef.current?.enqueue(ops);
  }, []);

  // Run a command: apply locally, persist, record on the undo stack, drop redo.
  const run = useCallback(
    (cmd: EditCommand) => {
      if (cmd.ops.length === 0) return;
      setRegions((rs) => applyOps(rs, cmd.ops));
      persistOps(cmd.ops);
      setPast([...pastRef.current, cmd].slice(-HISTORY_CAP));
      setFuture([]);
    },
    [persistOps],
  );

  const undo = useCallback(() => {
    const p = pastRef.current;
    if (p.length === 0) return;
    const cmd = p[p.length - 1];
    const inv = invertOps(cmd.ops);
    setRegions((rs) => applyOps(rs, inv));
    persistOps(inv);
    setPast(p.slice(0, -1));
    setFuture([cmd, ...futureRef.current]);
  }, [persistOps]);

  const redo = useCallback(() => {
    const f = futureRef.current;
    if (f.length === 0) return;
    const cmd = f[0];
    setRegions((rs) => applyOps(rs, cmd.ops));
    persistOps(cmd.ops);
    setFuture(f.slice(1));
    setPast([...pastRef.current, cmd].slice(-HISTORY_CAP));
  }, [persistOps]);

  // Move / trim / fade come up from RegionBlock as a finished region.
  const commitRegion = useCallback(
    (next: Region) => {
      const before = regionsRef.current.find((r) => r.id === next.id);
      if (!before) return;
      run({
        label: "Edit clip",
        ops: [{ kind: "update", before, after: next }],
      });
    },
    [run],
  );

  const deleteRegion = useCallback(
    (id: string) => {
      const target = regionsRef.current.find((r) => r.id === id);
      if (!target) return;
      select(null);
      run({ label: "Delete clip", ops: [{ kind: "delete", region: target }] });
    },
    [run, select],
  );

  const rippleDelete = useCallback(
    (id: string) => {
      const target = regionsRef.current.find((r) => r.id === id);
      if (!target) return;
      select(null);
      run({
        label: "Ripple delete",
        ops: rippleDeleteOps(regionsRef.current, target),
      });
    },
    [run, select],
  );

  const splitSelected = useCallback(() => {
    if (!selectedRegionId) return;
    const target = regionsRef.current.find((r) => r.id === selectedRegionId);
    if (!target) return;
    const res = splitRegion(target, playheadMs, crypto.randomUUID());
    if (!res) return; // playhead not strictly inside the clip
    run({
      label: "Split",
      ops: [
        { kind: "update", before: target, after: res.left },
        { kind: "create", region: res.right },
      ],
    });
    select(res.right.id);
  }, [run, select, selectedRegionId, playheadMs]);

  const adjustGain = useCallback(
    (id: string, deltaDb: number) => {
      const before = regionsRef.current.find((r) => r.id === id);
      if (!before) return;
      const gain = clamp(before.gain_adjust_db + deltaDb, GAIN_MIN, GAIN_MAX);
      if (gain === before.gain_adjust_db) return;
      run({
        label: "Gain",
        ops: [
          {
            kind: "update",
            before,
            after: { ...before, gain_adjust_db: gain },
          },
        ],
      });
    },
    [run],
  );

  const mergeWithNext = useCallback(
    (id: string) => {
      const target = regionsRef.current.find((r) => r.id === id);
      if (!target) return;
      const next = mergeableNext(regionsRef.current, target);
      if (!next) return;
      run({ label: "Merge", ops: mergeOps(target, next) });
      select(target.id);
    },
    [run, select],
  );

  const crossfadePrev = useCallback(
    (id: string) => {
      const target = regionsRef.current.find((r) => r.id === id);
      if (!target) return;
      const ov = overlapWithPrev(regionsRef.current, target);
      if (!ov) return;
      run({
        label: "Crossfade",
        ops: crossfadeOps(ov.prev, target, ov.overlapMs),
      });
    },
    [run],
  );

  const copyRegion = useCallback((id: string) => {
    const r = regionsRef.current.find((x) => x.id === id);
    if (!r) return;
    clipboardRef.current = r;
    setHasClipboard(true);
  }, []);

  const cutRegion = useCallback(
    (id: string) => {
      copyRegion(id);
      deleteRegion(id);
    },
    [copyRegion, deleteRegion],
  );

  const pasteClipboard = useCallback(() => {
    const clip = clipboardRef.current;
    if (!clip) return;
    const pasted = pasteRegion(clip, crypto.randomUUID(), playheadMs);
    run({ label: "Paste", ops: [{ kind: "create", region: pasted }] });
    select(pasted.id);
  }, [run, select, playheadMs]);

  const removeSilence = useCallback(
    async (id: string) => {
      const target = regionsRef.current.find((r) => r.id === id);
      if (!target) return;
      setError(null);
      setInfo(null);
      setAnalyzing(true);
      try {
        const spans = await ipc.edit.analyzeSilence(
          target.take_id,
          target.source_track_id,
          SILENCE_DB,
          MIN_SILENCE_MS,
        );
        const ops = removeSilencesOps(target, spans, () => crypto.randomUUID());
        if (ops.length === 0) {
          setInfo("No silences over 0.5s found in this clip.");
          return;
        }
        const before = target.end_in_take_ms - target.start_in_take_ms;
        const keptTotal = ops.reduce(
          (a, o) =>
            o.kind === "create"
              ? a + (o.region.end_in_take_ms - o.region.start_in_take_ms)
              : a,
          0,
        );
        const clips = ops.filter((o) => o.kind === "create").length;
        run({ label: "Remove silences", ops });
        select(null);
        setInfo(
          `Removed silence — ${((before - keptTotal) / 1000).toFixed(1)}s shorter, now ${clips} clip(s).`,
        );
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      } finally {
        setAnalyzing(false);
      }
    },
    [run, select],
  );

  // Bundled voice-processing presets, for the per-track FX picker.
  const voicePresets = useQuery({
    queryKey: ["dsp_presets"],
    queryFn: () => ipc.dsp.presets(),
    staleTime: Infinity,
  });

  const setTrackVoicePreset = useCallback(
    async (track: Track, presetId: string | null) => {
      try {
        await ipc.project.updateTrack({ ...track, voice_preset: presetId });
        setSnapshot(await ipc.project.snapshot());
      } catch (e) {
        setError(e instanceof Error ? e.message : String(e));
      }
    },
    [setSnapshot],
  );

  // AI auto-leveling (Pro): ask Claude for per-track gains, then apply them to
  // the tracks. Network I/O runs on a blocking thread backend-side; we just wait
  // for the suggestions and write them to each track's fader gain.
  const autoLevel = useCallback(async () => {
    setError(null);
    setInfo(null);
    setLeveling(true);
    try {
      const result = await ipc.ai.autoLevel();
      if (result.suggestions.length === 0) {
        setInfo(
          "AI had no leveling changes to suggest — the mix looks balanced.",
        );
        return;
      }
      const byId = new Map(snapshot?.tracks.map((t) => [t.id, t]) ?? []);
      let applied = 0;
      for (const s of result.suggestions) {
        const track = byId.get(s.track_id);
        if (!track || track.gain_db === s.suggested_gain_db) continue;
        await ipc.project.updateTrack({
          ...track,
          gain_db: s.suggested_gain_db,
        });
        applied += 1;
      }
      if (applied > 0) setSnapshot(await ipc.project.snapshot());
      setInfo(
        `AI auto-level applied gain to ${applied} track${applied === 1 ? "" : "s"}.`,
      );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLeveling(false);
    }
  }, [snapshot, setSnapshot]);

  // Keyboard: edit ops + clipboard + undo/redo (skip while typing in a field).
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target as HTMLElement | null;
      if (el && /^(INPUT|TEXTAREA)$/.test(el.tagName)) return;
      const mod = e.metaKey || e.ctrlKey;
      const key = e.key.toLowerCase();

      if (mod && key === "z") {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
        return;
      }
      if (mod && key === "y") {
        e.preventDefault();
        redo();
        return;
      }
      if (mod && key === "c" && selectedRegionId) {
        e.preventDefault();
        copyRegion(selectedRegionId);
        return;
      }
      if (mod && key === "x" && selectedRegionId) {
        e.preventDefault();
        cutRegion(selectedRegionId);
        return;
      }
      if (mod && key === "v") {
        e.preventDefault();
        pasteClipboard();
        return;
      }
      if (!mod && key === "s") {
        e.preventDefault();
        splitSelected();
        return;
      }
      if (!mod && key === "m" && selectedRegionId) {
        e.preventDefault();
        mergeWithNext(selectedRegionId);
        return;
      }
      if ((e.key === "Delete" || e.key === "Backspace") && selectedRegionId) {
        e.preventDefault();
        if (e.shiftKey) rippleDelete(selectedRegionId);
        else deleteRegion(selectedRegionId);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [
    selectedRegionId,
    undo,
    redo,
    splitSelected,
    deleteRegion,
    rippleDelete,
    mergeWithNext,
    copyRegion,
    cutRegion,
    pasteClipboard,
  ]);

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
      // A fresh import is a new baseline — past edits no longer apply cleanly.
      setPast([]);
      setFuture([]);
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
  const selected = selectedRegionId
    ? regions.find((r) => r.id === selectedRegionId)
    : undefined;
  const canSplit =
    !!selected &&
    playheadMs > selected.position_in_timeline_ms &&
    playheadMs <
      selected.position_in_timeline_ms +
        (selected.end_in_take_ms - selected.start_in_take_ms);
  const canMerge = !!selected && !!mergeableNext(regions, selected);
  const canCrossfade = !!selected && !!overlapWithPrev(regions, selected);

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
          <Button
            variant="surface"
            size="sm"
            onClick={autoLevel}
            disabled={leveling || !hasContent}
            title="Let AI balance the track levels (Sunday Cast Pro)"
          >
            {leveling ? (
              <Loader2 size={15} className="animate-spin" />
            ) : (
              <Sparkles size={15} />
            )}
            {leveling ? "Leveling…" : "Auto-level"}
          </Button>
          <div className="mx-1 h-5 w-px bg-[var(--color-border)]" />
          <Button
            variant="ghost"
            size="sm"
            onClick={undo}
            disabled={past.length === 0}
            aria-label="Undo"
            title="Undo (⌘Z)"
          >
            <Undo2 size={15} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={redo}
            disabled={future.length === 0}
            aria-label="Redo"
            title="Redo (⌘⇧Z)"
          >
            <Redo2 size={15} />
          </Button>
          <Button
            variant="ghost"
            size="sm"
            onClick={pasteClipboard}
            disabled={!hasClipboard}
            aria-label="Paste"
            title="Paste clip at playhead (⌘V)"
          >
            <ClipboardPaste size={15} />
          </Button>
          <div className="mx-1 h-5 w-px bg-[var(--color-border)]" />
          <Button
            variant={snapEnabled ? "accent" : "ghost"}
            size="sm"
            onClick={toggleSnap}
            aria-pressed={snapEnabled}
            title="Snap to edges, markers and playhead"
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
      {info && (
        <div className="flex items-start gap-2 border-b border-[var(--color-border)] bg-[var(--color-bg-surface)] px-5 py-2 text-ui-xs text-[var(--color-fg-muted)]">
          <Eraser size={14} className="mt-0.5 shrink-0" />
          <span>{info}</span>
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
                className="flex flex-col justify-center gap-1 border-b border-[var(--color-border)]/60 px-3"
                style={{ height: LANE_H }}
              >
                <div className="flex items-center gap-2">
                  <span
                    className="size-2.5 shrink-0 rounded-full"
                    style={{ background: t.color }}
                  />
                  <span className="truncate text-ui-sm">{t.name}</span>
                </div>
                <select
                  value={t.voice_preset ?? ""}
                  onChange={(e) =>
                    setTrackVoicePreset(t, e.target.value || null)
                  }
                  title="Voice processing applied to this track on export"
                  className="w-full rounded-[var(--radius-xs)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-1.5 py-0.5 text-[11px] text-[var(--color-fg-muted)]"
                >
                  <option value="">No FX</option>
                  {(voicePresets.data ?? []).map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </select>
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
      {selected && (
        <div className="flex items-center justify-between gap-4 border-t border-[var(--color-border)] bg-[var(--color-bg-elevated)] px-5 py-2 text-ui-xs">
          <div className="flex items-center gap-3">
            <Button
              variant="surface"
              size="sm"
              onClick={splitSelected}
              disabled={!canSplit}
              title="Split at playhead (S)"
            >
              <Scissors size={14} />
              Split
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => mergeWithNext(selected.id)}
              disabled={!canMerge}
              title="Merge with the next clip (M)"
            >
              <Combine size={14} />
              Merge
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => crossfadePrev(selected.id)}
              disabled={!canCrossfade}
              title="Crossfade with the overlapping clip"
            >
              <Blend size={14} />
              Crossfade
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => removeSilence(selected.id)}
              disabled={analyzing}
              title="Detect and remove silent gaps in this clip"
            >
              {analyzing ? (
                <Loader2 size={14} className="animate-spin" />
              ) : (
                <Eraser size={14} />
              )}
              Remove silence
            </Button>

            {/* Per-clip gain */}
            <div className="flex items-center gap-1.5">
              <Volume2 size={14} className="text-[var(--color-fg-muted)]" />
              <Button
                variant="ghost"
                size="sm"
                onClick={() => adjustGain(selected.id, -GAIN_STEP)}
                aria-label="Decrease gain"
              >
                <Minus size={13} />
              </Button>
              <span className="w-14 text-center font-mono tabular-nums">
                {selected.gain_adjust_db > 0 ? "+" : ""}
                {selected.gain_adjust_db.toFixed(1)} dB
              </span>
              <Button
                variant="ghost"
                size="sm"
                onClick={() => adjustGain(selected.id, GAIN_STEP)}
                aria-label="Increase gain"
              >
                <Plus size={13} />
              </Button>
            </div>

            <span className="text-[var(--color-fg-muted)]">
              Drag to move · edge to trim · corner to fade
            </span>
          </div>

          <div className="flex items-center gap-1">
            <Button
              variant="ghost"
              size="sm"
              onClick={() => copyRegion(selected.id)}
              title="Copy clip (⌘C)"
            >
              <Copy size={14} />
              Copy
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => cutRegion(selected.id)}
              title="Cut clip (⌘X)"
            >
              <SquareScissors size={14} />
              Cut
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => rippleDelete(selected.id)}
              title="Delete and close the gap (⇧⌫)"
            >
              <FoldHorizontal size={14} />
              Ripple delete
            </Button>
            <Button
              variant="ghost"
              size="sm"
              onClick={() => deleteRegion(selected.id)}
              title="Delete clip (⌫)"
            >
              <Trash2 size={14} />
              Delete
            </Button>
          </div>
        </div>
      )}
    </div>
  );
}

function clamp(v: number, lo: number, hi: number): number {
  return Math.min(hi, Math.max(lo, v));
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
