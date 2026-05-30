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
