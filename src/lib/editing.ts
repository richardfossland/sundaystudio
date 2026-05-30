/**
 * Non-destructive editing operations (Phase 3.2) as a small command algebra.
 *
 * Every edit is expressed as a list of primitive region ops (create / update /
 * delete), each with a trivially computable inverse. That gives undo/redo for
 * free: to undo a command, run the inverted ops in reverse. The same ops drive
 * both the local timeline state and the backend persistence, so the two never
 * drift. These functions are pure — the EditPage wires them to IPC + state.
 */
import type { Region } from "./bindings";

export type PrimOp =
  | { kind: "create"; region: Region }
  | { kind: "delete"; region: Region }
  | { kind: "update"; before: Region; after: Region };

export interface EditCommand {
  label: string;
  ops: PrimOp[];
}

/** Anti-click fade applied to a freshly cut edge, in ms. */
export const CUT_FADE_MS = 5;

/** Invert a command's ops: reverse order, swap each op for its undo. */
export function invertOps(ops: PrimOp[]): PrimOp[] {
  return [...ops].reverse().map((op): PrimOp => {
    if (op.kind === "create") return { kind: "delete", region: op.region };
    if (op.kind === "delete") return { kind: "create", region: op.region };
    return { kind: "update", before: op.after, after: op.before };
  });
}

/** Apply a list of ops to a regions array (pure — returns a new array). */
export function applyOps(regions: Region[], ops: PrimOp[]): Region[] {
  let next = regions;
  for (const op of ops) {
    if (op.kind === "create") {
      next = [...next, op.region];
    } else if (op.kind === "delete") {
      next = next.filter((r) => r.id !== op.region.id);
    } else {
      next = next.map((r) => (r.id === op.after.id ? op.after : r));
    }
  }
  return next;
}

/** Timeline length a region occupies (= its trimmed take window). */
export function regionDurationMs(r: Region): number {
  return r.end_in_take_ms - r.start_in_take_ms;
}

/** A region's right edge on the timeline. */
export function regionEndMs(r: Region): number {
  return r.position_in_timeline_ms + regionDurationMs(r);
}

/**
 * Split a region at a timeline position into a shortened left (the original,
 * updated) and a new right region. Returns null if the playhead isn't strictly
 * inside the region. The inner edges get a short anti-click fade; the outer
 * fades are preserved.
 */
export function splitRegion(
  region: Region,
  playheadMs: number,
  newRightId: string,
): { left: Region; right: Region } | null {
  const start = region.position_in_timeline_ms;
  const end = regionEndMs(region);
  if (playheadMs <= start || playheadMs >= end) return null;

  const cutInTake = region.start_in_take_ms + (playheadMs - start);
  const left: Region = {
    ...region,
    end_in_take_ms: cutInTake,
    fade_out_ms: CUT_FADE_MS,
  };
  const right: Region = {
    ...region,
    id: newRightId,
    start_in_take_ms: cutInTake,
    position_in_timeline_ms: playheadMs,
    fade_in_ms: CUT_FADE_MS,
  };
  return { left, right };
}

/** Clips within this many ms count as contiguous (rounding tolerance). */
const CONTIGUOUS_EPS_MS = 1;

/**
 * The clip immediately after `region` on the same track that continues the same
 * source take with no gap — i.e. the inverse of a split. Returns null if there's
 * nothing cleanly mergeable (merging only makes sense for one continuous source).
 */
export function mergeableNext(
  regions: Region[],
  region: Region,
): Region | null {
  const end = regionEndMs(region);
  return (
    regions.find(
      (r) =>
        r.id !== region.id &&
        r.target_track_id === region.target_track_id &&
        r.take_id === region.take_id &&
        r.source_track_id === region.source_track_id &&
        Math.abs(r.position_in_timeline_ms - end) <= CONTIGUOUS_EPS_MS &&
        Math.abs(r.start_in_take_ms - region.end_in_take_ms) <=
          CONTIGUOUS_EPS_MS,
    ) ?? null
  );
}

/** Ops to merge `region` with its contiguous next clip into a single region. */
export function mergeOps(region: Region, next: Region): PrimOp[] {
  return [
    {
      kind: "update",
      before: region,
      after: {
        ...region,
        end_in_take_ms: next.end_in_take_ms,
        fade_out_ms: next.fade_out_ms,
      },
    },
    { kind: "delete", region: next },
  ];
}

/**
 * If `region` overlaps the clip just before it on the same track, return that
 * clip and the overlap length; else null. Used to offer a crossfade.
 */
export function overlapWithPrev(
  regions: Region[],
  region: Region,
): { prev: Region; overlapMs: number } | null {
  let prev: Region | null = null;
  for (const r of regions) {
    if (r.id === region.id || r.target_track_id !== region.target_track_id) {
      continue;
    }
    if (r.position_in_timeline_ms <= region.position_in_timeline_ms) {
      if (!prev || r.position_in_timeline_ms > prev.position_in_timeline_ms) {
        prev = r;
      }
    }
  }
  if (!prev) return null;
  const overlapMs = regionEndMs(prev) - region.position_in_timeline_ms;
  return overlapMs > 0 ? { prev, overlapMs } : null;
}

/**
 * Ops to crossfade `region` with the overlapping previous clip: fade the earlier
 * clip out and this one in across the overlap. (Linear fades — equal-power is a
 * later refinement; for voice the difference is inaudible.)
 */
export function crossfadeOps(
  prev: Region,
  region: Region,
  overlapMs: number,
): PrimOp[] {
  return [
    {
      kind: "update",
      before: prev,
      after: { ...prev, fade_out_ms: overlapMs },
    },
    {
      kind: "update",
      before: region,
      after: { ...region, fade_in_ms: overlapMs },
    },
  ];
}

/** A take-relative silent span (mirrors the backend SilenceSpan). */
export interface Span {
  start_ms: number;
  end_ms: number;
}

/** The kept (non-silent) take-time spans of a region after removing `silences`,
 *  clipped to the region's window. Pure — drives both the preview and the ops. */
export function keptSpans(region: Region, silences: Span[]): Span[] {
  const s0 = region.start_in_take_ms;
  const e0 = region.end_in_take_ms;
  const sil = silences
    .map((s) => ({
      start_ms: Math.max(s0, s.start_ms),
      end_ms: Math.min(e0, s.end_ms),
    }))
    .filter((s) => s.end_ms > s.start_ms)
    .sort((a, b) => a.start_ms - b.start_ms);

  const kept: Span[] = [];
  let cursor = s0;
  for (const s of sil) {
    if (s.start_ms > cursor)
      kept.push({ start_ms: cursor, end_ms: s.start_ms });
    cursor = Math.max(cursor, s.end_ms);
  }
  if (cursor < e0) kept.push({ start_ms: cursor, end_ms: e0 });
  return kept;
}

/**
 * Ops to remove the silent spans from a region: replace it with one clip per
 * kept span, ripple-packed from the region's position so there are no gaps. The
 * original outer fades are preserved on the first/last clip; inner cuts get a
 * short anti-click fade. Returns [] when there's nothing silent to remove.
 */
export function removeSilencesOps(
  region: Region,
  silences: Span[],
  mintId: () => string,
): PrimOp[] {
  const kept = keptSpans(region, silences);
  // Nothing silent inside the window → leave the clip untouched.
  if (
    kept.length === 1 &&
    kept[0].start_ms === region.start_in_take_ms &&
    kept[0].end_ms === region.end_in_take_ms
  ) {
    return [];
  }

  const ops: PrimOp[] = [{ kind: "delete", region }];
  let pos = region.position_in_timeline_ms;
  kept.forEach((k, i) => {
    ops.push({
      kind: "create",
      region: {
        ...region,
        id: mintId(),
        start_in_take_ms: k.start_ms,
        end_in_take_ms: k.end_ms,
        position_in_timeline_ms: pos,
        fade_in_ms: i === 0 ? region.fade_in_ms : CUT_FADE_MS,
        fade_out_ms: i === kept.length - 1 ? region.fade_out_ms : CUT_FADE_MS,
      },
    });
    pos += k.end_ms - k.start_ms;
  });
  return ops;
}

/** A pasted copy of a clip: same source/trim/fades/gain, new id + position. */
export function pasteRegion(
  clip: Region,
  newId: string,
  positionMs: number,
): Region {
  return {
    ...clip,
    id: newId,
    position_in_timeline_ms: Math.max(0, positionMs),
  };
}

/**
 * Ops for a ripple delete: remove `target` and pull every later clip on the same
 * track earlier by the gap it left, so there's no hole. Clips before the target
 * are untouched.
 */
export function rippleDeleteOps(regions: Region[], target: Region): PrimOp[] {
  const ops: PrimOp[] = [{ kind: "delete", region: target }];
  const gap = regionDurationMs(target);
  const from = target.position_in_timeline_ms;
  for (const r of regions) {
    if (r.id === target.id || r.target_track_id !== target.target_track_id) {
      continue;
    }
    if (r.position_in_timeline_ms >= from) {
      ops.push({
        kind: "update",
        before: r,
        after: {
          ...r,
          position_in_timeline_ms: Math.max(0, r.position_in_timeline_ms - gap),
        },
      });
    }
  }
  return ops;
}
