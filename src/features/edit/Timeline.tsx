import { useMemo, useRef } from "react";

import { msToPx, pxToMs, rulerTicks } from "@/lib/timeline";
import type { Marker, Region, Track } from "@/lib/bindings";

import { RegionBlock } from "./RegionBlock";

/** Row geometry, shared with the EditPage header column so rows line up. */
export const RULER_H = 28;
export const LANE_H = 64;
/** Trailing empty space after the last clip, so there's room to drag/extend. */
const TRAILING_MS = 5_000;
const MIN_CONTENT_MS = 30_000;

/**
 * The scrollable timeline: ruler, one lane per track, region clips, markers, and
 * the playhead. Pure presentation over the regions it's handed — all edits flow
 * back up through `onCommitRegion` / `onSeek` / `onSelect`.
 */
export function Timeline({
  tracks,
  regions,
  markers,
  pxPerSec,
  playheadMs,
  selectedRegionId,
  snapEnabled,
  onSeek,
  onSelect,
  onCommitRegion,
}: {
  tracks: Track[];
  regions: Region[];
  markers: Marker[];
  pxPerSec: number;
  playheadMs: number;
  selectedRegionId: string | null;
  snapEnabled: boolean;
  onSeek: (ms: number) => void;
  onSelect: (id: string | null) => void;
  onCommitRegion: (region: Region) => void;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);

  const contentMs = useMemo(() => {
    const end = regions.reduce((max, r) => {
      const right =
        r.position_in_timeline_ms + (r.end_in_take_ms - r.start_in_take_ms);
      return Math.max(max, right);
    }, 0);
    return Math.max(MIN_CONTENT_MS, end + TRAILING_MS);
  }, [regions]);

  const contentWidth = msToPx(contentMs, pxPerSec);
  const ticks = useMemo(
    () => rulerTicks(contentMs, pxPerSec),
    [contentMs, pxPerSec],
  );

  // All snap targets in timeline space: origin, playhead, markers, region edges.
  const allEdges = useMemo(() => {
    const edges: number[] = [0, playheadMs];
    for (const m of markers) edges.push(m.position_ms);
    for (const r of regions) {
      edges.push(r.position_in_timeline_ms);
      edges.push(
        r.position_in_timeline_ms + (r.end_in_take_ms - r.start_in_take_ms),
      );
    }
    return edges;
  }, [markers, regions, playheadMs]);

  function seekFromEvent(e: React.PointerEvent) {
    const host = scrollRef.current;
    if (!host) return;
    const rect = host.getBoundingClientRect();
    const x = e.clientX - rect.left + host.scrollLeft;
    onSeek(Math.max(0, pxToMs(x, pxPerSec)));
  }

  const playheadX = msToPx(playheadMs, pxPerSec);

  return (
    <div ref={scrollRef} className="relative flex-1 overflow-auto">
      <div className="relative" style={{ width: contentWidth }}>
        {/* Ruler */}
        <div
          className="sticky top-0 z-20 cursor-text border-b border-[var(--color-border)] bg-[var(--color-bg-elevated)]"
          style={{ height: RULER_H }}
          onPointerDown={seekFromEvent}
        >
          {ticks.map((t) => (
            <div
              key={t.ms}
              className="absolute top-0 flex h-full items-center"
              style={{ left: msToPx(t.ms, pxPerSec) }}
            >
              <div className="absolute left-0 top-0 h-2 w-px bg-[var(--color-border)]" />
              <span className="ml-1 font-mono text-[10px] text-[var(--color-fg-muted)]">
                {t.label}
              </span>
            </div>
          ))}
        </div>

        {/* Lanes */}
        <div
          onPointerDown={(e) => {
            // Clicking empty lane area seeks and clears selection.
            seekFromEvent(e);
            onSelect(null);
          }}
        >
          {tracks.map((track) => {
            const laneRegions = regions.filter(
              (r) => r.target_track_id === track.id,
            );
            return (
              <div
                key={track.id}
                className="relative border-b border-[var(--color-border)]/60"
                style={{ height: LANE_H }}
              >
                {laneRegions.map((r) => (
                  <RegionBlock
                    key={r.id}
                    region={r}
                    pxPerSec={pxPerSec}
                    color={track.color}
                    selected={r.id === selectedRegionId}
                    snapEnabled={snapEnabled}
                    snapTargets={excludeOwnEdges(allEdges, r)}
                    onSelect={() => onSelect(r.id)}
                    onCommit={onCommitRegion}
                  />
                ))}
              </div>
            );
          })}
        </div>

        {/* Markers (chapters) */}
        {markers.map((m) => (
          <div
            key={m.id}
            className="pointer-events-none absolute z-10"
            style={{
              left: msToPx(m.position_ms, pxPerSec),
              top: RULER_H,
              bottom: 0,
            }}
          >
            <div className="h-full w-px bg-[var(--color-fg-muted)]/40" />
            <div
              className="absolute -left-1 top-0 size-2 rounded-full"
              style={{ background: m.color }}
            />
          </div>
        ))}

        {/* Playhead */}
        <div
          className="pointer-events-none absolute top-0 bottom-0 z-30 w-px bg-[var(--color-accent)]"
          style={{ left: playheadX }}
        >
          <div className="absolute -left-[3px] top-0 size-0 border-x-[4px] border-t-[6px] border-x-transparent border-t-[var(--color-accent)]" />
        </div>
      </div>
    </div>
  );
}

/** Snap targets for a region exclude its own committed edges (no self-stick). */
function excludeOwnEdges(edges: number[], r: Region): number[] {
  const own = new Set([
    r.position_in_timeline_ms,
    r.position_in_timeline_ms + (r.end_in_take_ms - r.start_in_take_ms),
  ]);
  return edges.filter((e) => !own.has(e));
}
