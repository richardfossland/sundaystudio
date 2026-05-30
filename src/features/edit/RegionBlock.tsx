import { useState } from "react";
import { useQuery } from "@tanstack/react-query";

import { Waveform } from "@/components/audio";
import { cn } from "@/lib/cn";
import { ipc } from "@/lib/ipc";
import { msToPx, pxToMs, snap } from "@/lib/timeline";
import type { Region } from "@/lib/bindings";

type DragMode = "move" | "trim-l" | "trim-r" | "fade-l" | "fade-r";

/** Minimum clip length, in ms — keeps a region grabbable and audible. */
const MIN_REGION_MS = 20;
/** Snap tolerance scales with zoom: ~8px feels right at any zoom. */
const SNAP_PX = 8;

interface Draft {
  positionMs: number;
  startMs: number;
  endMs: number;
  fadeInMs: number;
  fadeOutMs: number;
}

/**
 * One clip on the timeline. Renders the take's waveform slice and supports the
 * direct-manipulation edits: drag the body to move, drag an edge to trim, drag a
 * top corner to set a fade, click to select. Edits preview live (local draft)
 * and commit once on release via `onCommit`, which the EditPage routes through
 * the undo/redo stack.
 */
export function RegionBlock({
  region,
  pxPerSec,
  color,
  selected,
  snapEnabled,
  snapTargets,
  onSelect,
  onCommit,
}: {
  region: Region;
  pxPerSec: number;
  color: string;
  selected: boolean;
  snapEnabled: boolean;
  /** Timeline-space ms to snap edges to (other region edges, markers, playhead). */
  snapTargets: number[];
  onSelect: () => void;
  onCommit: (next: Region) => void;
}) {
  const [draft, setDraft] = useState<Draft | null>(null);

  // The take track's waveform overview (immutable per take track → cache hard).
  const peaksQuery = useQuery({
    queryKey: ["peaks", region.take_id, region.source_track_id],
    queryFn: () => ipc.edit.peaks(region.take_id, region.source_track_id),
    staleTime: Infinity,
    gcTime: Infinity,
  });

  const base: Draft = {
    positionMs: region.position_in_timeline_ms,
    startMs: region.start_in_take_ms,
    endMs: region.end_in_take_ms,
    fadeInMs: region.fade_in_ms,
    fadeOutMs: region.fade_out_ms,
  };
  const live = draft ?? base;
  const durationMs = Math.max(MIN_REGION_MS, live.endMs - live.startMs);
  const leftPx = msToPx(live.positionMs, pxPerSec);
  const width = msToPx(durationMs, pxPerSec);
  const takeDurationMs =
    peaksQuery.data?.duration_ms ?? Number.POSITIVE_INFINITY;

  // A drag is self-contained: it captures the zoom/snap context at the moment it
  // starts (these don't change mid-drag) and binds its own window listeners, so
  // there's no effect-timing dance. The latest draft is tracked in a closure
  // variable and committed once on release.
  function beginDrag(mode: DragMode, e: React.PointerEvent) {
    e.preventDefault();
    e.stopPropagation();
    onSelect();

    const startX = e.clientX;
    const tolMs = pxToMs(SNAP_PX, pxPerSec);
    const maybeSnap = (ms: number) =>
      snapEnabled ? snap(ms, snapTargets, tolMs) : ms;
    let latest = base;

    const onMove = (ev: PointerEvent) => {
      const deltaMs = pxToMs(ev.clientX - startX, pxPerSec);
      const dur = base.endMs - base.startMs;

      if (mode === "move") {
        const positionMs = Math.max(0, maybeSnap(base.positionMs + deltaMs));
        latest = { ...base, positionMs };
      } else if (mode === "trim-l") {
        // Drag the left edge: snap its timeline position, then move start +
        // position together so the audio under the edge stays put.
        const edge = Math.max(0, maybeSnap(base.positionMs + deltaMs));
        let shift = edge - base.positionMs;
        // Clamp so start stays ≥ 0 and the clip keeps a minimum length.
        shift = Math.max(shift, -base.startMs);
        shift = Math.min(shift, dur - MIN_REGION_MS);
        latest = {
          ...base,
          positionMs: base.positionMs + shift,
          startMs: base.startMs + shift,
        };
      } else if (mode === "trim-r") {
        // Drag the right edge: snap its timeline position, then move end only.
        const rightEdge = maybeSnap(base.positionMs + dur + deltaMs);
        let endMs = base.startMs + (rightEdge - base.positionMs);
        endMs = Math.max(endMs, base.startMs + MIN_REGION_MS);
        endMs = Math.min(endMs, takeDurationMs);
        latest = { ...base, endMs };
      } else if (mode === "fade-l") {
        // Drag the top-left grip right to lengthen the fade-in (no snap).
        const fadeInMs = clampFade(base.fadeInMs + deltaMs, dur);
        latest = { ...base, fadeInMs };
      } else {
        // Drag the top-right grip left to lengthen the fade-out.
        const fadeOutMs = clampFade(base.fadeOutMs - deltaMs, dur);
        latest = { ...base, fadeOutMs };
      }
      setDraft(latest);
    };

    const onUp = () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      setDraft(null);
      // A pure click (no movement) leaves the clip unchanged — don't persist.
      const changed =
        latest.positionMs !== base.positionMs ||
        latest.startMs !== base.startMs ||
        latest.endMs !== base.endMs ||
        latest.fadeInMs !== base.fadeInMs ||
        latest.fadeOutMs !== base.fadeOutMs;
      if (changed) {
        onCommit({
          ...region,
          position_in_timeline_ms: latest.positionMs,
          start_in_take_ms: latest.startMs,
          end_in_take_ms: latest.endMs,
          fade_in_ms: latest.fadeInMs,
          fade_out_ms: latest.fadeOutMs,
        });
      }
    };

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    setDraft(base);
  }

  // Slice the take's peaks to this region's [start,end] window.
  const peakSlice = slicePeaks(peaksQuery.data, live.startMs, live.endMs);
  const fadeInPct = clampPct((live.fadeInMs / durationMs) * 100);
  const fadeOutPct = clampPct((live.fadeOutMs / durationMs) * 100);

  return (
    <div
      role="button"
      tabIndex={0}
      onPointerDown={(e) => beginDrag("move", e)}
      onClick={(e) => {
        e.stopPropagation();
        onSelect();
      }}
      className={cn(
        "absolute top-1 bottom-1 cursor-grab touch-none select-none overflow-hidden rounded-[var(--radius-sm)] border active:cursor-grabbing",
        selected
          ? "border-[var(--color-accent)] ring-1 ring-[var(--color-accent)]"
          : "border-[var(--color-border)]",
      )}
      style={{
        left: leftPx,
        width: Math.max(2, width),
        background: `color-mix(in oklab, ${color} 22%, var(--color-bg-elevated))`,
      }}
    >
      {/* Waveform */}
      <div className="pointer-events-none absolute inset-0 px-px py-1">
        {peakSlice.length > 0 ? (
          <Waveform peaks={peakSlice} className="h-full" color={color} />
        ) : (
          <div className="h-full" />
        )}
      </div>

      {/* Fade overlays */}
      {fadeInPct > 0 && (
        <div
          className="pointer-events-none absolute inset-y-0 left-0 bg-gradient-to-r from-[var(--color-bg)]/55 to-transparent"
          style={{ width: `${fadeInPct}%` }}
        />
      )}
      {fadeOutPct > 0 && (
        <div
          className="pointer-events-none absolute inset-y-0 right-0 bg-gradient-to-l from-[var(--color-bg)]/55 to-transparent"
          style={{ width: `${fadeOutPct}%` }}
        />
      )}

      {/* Trim handles (full-height edges) */}
      <div
        onPointerDown={(e) => beginDrag("trim-l", e)}
        className="absolute inset-y-0 left-0 w-1.5 cursor-ew-resize hover:bg-[var(--color-accent)]/60"
      />
      <div
        onPointerDown={(e) => beginDrag("trim-r", e)}
        className="absolute inset-y-0 right-0 w-1.5 cursor-ew-resize hover:bg-[var(--color-accent)]/60"
      />

      {/* Fade grips (top corners) — only when selected, to stay uncluttered */}
      {selected && (
        <>
          <div
            onPointerDown={(e) => beginDrag("fade-l", e)}
            title="Drag to fade in"
            className="absolute left-1 top-0.5 size-2.5 cursor-ew-resize rounded-full border border-[var(--color-bg)] bg-[var(--color-accent)]"
          />
          <div
            onPointerDown={(e) => beginDrag("fade-r", e)}
            title="Drag to fade out"
            className="absolute right-1 top-0.5 size-2.5 cursor-ew-resize rounded-full border border-[var(--color-bg)] bg-[var(--color-accent)]"
          />
        </>
      )}
    </div>
  );
}

/** Clamp a fade length to [0, clip duration]. */
function clampFade(ms: number, durationMs: number): number {
  return Math.min(Math.max(0, ms), Math.max(0, durationMs));
}

function clampPct(p: number): number {
  if (!Number.isFinite(p)) return 0;
  return Math.min(50, Math.max(0, p));
}

/** Slice a take's full-length peaks to the region's [startMs,endMs] window. */
function slicePeaks(
  data:
    | { peaks: number[]; samples_per_peak: number; sample_rate: number }
    | undefined,
  startMs: number,
  endMs: number,
): number[] {
  if (!data || data.peaks.length === 0) return [];
  const msPerPeak =
    (data.samples_per_peak / Math.max(1, data.sample_rate)) * 1000;
  if (msPerPeak <= 0) return [];
  const from = Math.max(0, Math.floor(startMs / msPerPeak));
  const to = Math.min(data.peaks.length, Math.ceil(endMs / msPerPeak));
  return from < to ? data.peaks.slice(from, to) : [];
}
