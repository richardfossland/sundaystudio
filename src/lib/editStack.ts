/**
 * Undo/redo stack + higher-level edit commands on top of the `editing.ts`
 * primitive op algebra (Phase 3.2). Where `editing.ts` provides the leaf ops
 * (create / update / delete) and their inverses, this layer:
 *
 *   - bundles ops into named `EditCommand`s for common edits (trim, nudge,
 *     set-gain, duplicate, delete-in-place), each clamped to sane bounds;
 *   - holds a bounded undo/redo stack with the usual invariants (a fresh push
 *     clears the redo branch; depth is capped, dropping the oldest);
 *   - applies/undoes/redoes a command against a regions array.
 *
 * Everything is pure and immutable: `apply`, `undo`, and `redo` return a new
 * `{ regions, stack }` pair, never mutating their inputs. The EditPage owns one
 * `EditStack` in state and threads commands through it, so undo/redo and backend
 * persistence stay derived from the same op lists and never drift.
 */
import type { Region } from "./bindings";
import {
  applyOps,
  CUT_FADE_MS,
  invertOps,
  regionDurationMs,
  regionEndMs,
  type EditCommand,
  type PrimOp,
} from "./editing";

/** Default cap on undo depth (matches the SundayStudio editor memory budget). */
export const DEFAULT_MAX_DEPTH = 200;

/** Smallest clip we allow a trim to leave, so a region never collapses to 0. */
export const MIN_REGION_MS = 1;

/** An immutable undo/redo stack of applied commands. */
export interface EditStack {
  /** Commands already applied, oldest first; the last is the most recent. */
  readonly past: readonly EditCommand[];
  /** Commands undone and available to redo, most-recently-undone last. */
  readonly future: readonly EditCommand[];
  /** Maximum number of `past` entries retained. */
  readonly maxDepth: number;
}

/** A fresh, empty stack. */
export function emptyStack(maxDepth = DEFAULT_MAX_DEPTH): EditStack {
  return { past: [], future: [], maxDepth: Math.max(1, maxDepth) };
}

export function canUndo(stack: EditStack): boolean {
  return stack.past.length > 0;
}

export function canRedo(stack: EditStack): boolean {
  return stack.future.length > 0;
}

/**
 * Apply a command: run its ops against `regions`, push it onto `past`, and
 * clear the redo branch (a new edit forks history). Honours `maxDepth` by
 * dropping the oldest command when the cap is exceeded. A command with no ops
 * is a no-op: regions and stack are returned unchanged so trivial edits don't
 * pollute the undo history.
 */
export function apply(
  stack: EditStack,
  regions: Region[],
  command: EditCommand,
): { regions: Region[]; stack: EditStack } {
  if (command.ops.length === 0) {
    return { regions, stack };
  }
  const nextRegions = applyOps(regions, command.ops);
  let past = [...stack.past, command];
  if (past.length > stack.maxDepth) {
    past = past.slice(past.length - stack.maxDepth);
  }
  return { regions: nextRegions, stack: { ...stack, past, future: [] } };
}

/**
 * Undo the most recent command: invert its ops, apply them, move it to the
 * redo branch. No-op when there's nothing to undo.
 */
export function undo(
  stack: EditStack,
  regions: Region[],
): { regions: Region[]; stack: EditStack } {
  if (stack.past.length === 0) return { regions, stack };
  const command = stack.past[stack.past.length - 1];
  const nextRegions = applyOps(regions, invertOps(command.ops));
  return {
    regions: nextRegions,
    stack: {
      ...stack,
      past: stack.past.slice(0, -1),
      future: [...stack.future, command],
    },
  };
}

/**
 * Redo the most-recently-undone command: re-apply its ops and move it back to
 * `past`. No-op when there's nothing to redo.
 */
export function redo(
  stack: EditStack,
  regions: Region[],
): { regions: Region[]; stack: EditStack } {
  if (stack.future.length === 0) return { regions, stack };
  const command = stack.future[stack.future.length - 1];
  const nextRegions = applyOps(regions, command.ops);
  return {
    regions: nextRegions,
    stack: {
      ...stack,
      past: [...stack.past, command],
      future: stack.future.slice(0, -1),
    },
  };
}

// ── Command builders ────────────────────────────────────────────────────────
// Each returns an `EditCommand` (possibly with zero ops, meaning "no change").

/**
 * Move a clip along the timeline by `deltaMs`, clamped so it never starts before
 * 0. A pure timeline move — the take window is untouched. Returns a no-op
 * command when the clamped position is unchanged.
 */
export function nudgeCommand(region: Region, deltaMs: number): EditCommand {
  const target = Math.max(0, region.position_in_timeline_ms + deltaMs);
  if (target === region.position_in_timeline_ms) {
    return { label: "Nudge", ops: [] };
  }
  return {
    label: "Nudge",
    ops: [
      {
        kind: "update",
        before: region,
        after: { ...region, position_in_timeline_ms: target },
      },
    ],
  };
}

/**
 * Trim the clip's left edge to a new timeline position, moving its take-start in
 * lockstep so the audio under the kept portion doesn't slide. Clamped to the
 * take's available head room and to leaving at least `MIN_REGION_MS`. A trim in
 * gets a short anti-click fade on the new edge.
 */
export function trimStartCommand(
  region: Region,
  newStartMs: number,
): EditCommand {
  const right = regionEndMs(region);
  // Can't trim past the right edge (keep MIN_REGION_MS) and can't expand the
  // left beyond the take's available head (start_in_take_ms can't go below 0).
  const minStart = region.position_in_timeline_ms - region.start_in_take_ms; // exposes take head
  const maxStart = right - MIN_REGION_MS;
  const clamped = Math.min(maxStart, Math.max(minStart, newStartMs));
  const deltaMs = clamped - region.position_in_timeline_ms;
  if (deltaMs === 0) return { label: "Trim start", ops: [] };
  return {
    label: "Trim start",
    ops: [
      {
        kind: "update",
        before: region,
        after: {
          ...region,
          position_in_timeline_ms: clamped,
          start_in_take_ms: region.start_in_take_ms + deltaMs,
          fade_in_ms: CUT_FADE_MS,
        },
      },
    ],
  };
}

/**
 * Trim the clip's right edge to a new timeline position by shortening (or
 * extending, within the take) its take window. Clamped to at least
 * `MIN_REGION_MS` and to the take's available tail (`takeLengthMs`, when known —
 * pass `Infinity` if the take length isn't loaded). A trim gets a short
 * anti-click fade on the new edge.
 */
export function trimEndCommand(
  region: Region,
  newEndMs: number,
  takeLengthMs = Infinity,
): EditCommand {
  const minEnd = region.position_in_timeline_ms + MIN_REGION_MS;
  // The latest end is bounded by how much take is left after the clip's start.
  const maxByTake =
    region.position_in_timeline_ms + (takeLengthMs - region.start_in_take_ms);
  const clamped = Math.min(maxByTake, Math.max(minEnd, newEndMs));
  const newDuration = clamped - region.position_in_timeline_ms;
  const newTakeEnd = region.start_in_take_ms + newDuration;
  if (newTakeEnd === region.end_in_take_ms) {
    return { label: "Trim end", ops: [] };
  }
  return {
    label: "Trim end",
    ops: [
      {
        kind: "update",
        before: region,
        after: {
          ...region,
          end_in_take_ms: newTakeEnd,
          fade_out_ms: CUT_FADE_MS,
        },
      },
    ],
  };
}

/** Loudest sensible per-clip gain trim bounds, in dB. */
export const MIN_GAIN_DB = -24;
export const MAX_GAIN_DB = 12;

/**
 * Set a clip's per-region gain to `gainDb`, clamped to [MIN_GAIN_DB,
 * MAX_GAIN_DB]. No-op when the clamped value equals the current gain.
 */
export function setGainCommand(region: Region, gainDb: number): EditCommand {
  const clamped = Math.min(MAX_GAIN_DB, Math.max(MIN_GAIN_DB, gainDb));
  if (clamped === region.gain_adjust_db) {
    return { label: "Set gain", ops: [] };
  }
  return {
    label: "Set gain",
    ops: [
      {
        kind: "update",
        before: region,
        after: { ...region, gain_adjust_db: clamped },
      },
    ],
  };
}

/**
 * Duplicate a clip immediately after itself on the same track (new id, position
 * = the original's right edge). A create-only command, so its inverse is a clean
 * delete.
 */
export function duplicateCommand(region: Region, newId: string): EditCommand {
  const copy: Region = {
    ...region,
    id: newId,
    position_in_timeline_ms: regionEndMs(region),
  };
  return { label: "Duplicate", ops: [{ kind: "create", region: copy }] };
}

/** Delete a clip in place (leaving a gap). Inverse is a clean re-create. */
export function deleteCommand(region: Region): EditCommand {
  return { label: "Delete", ops: [{ kind: "delete", region }] };
}

/**
 * Compose several commands into one undoable unit (e.g. a multi-clip edit). The
 * ops are concatenated in order; inverting the combined op list (which
 * `undo` does) correctly reverses the whole batch. Commands with no ops are
 * dropped so an all-no-op batch stays a no-op.
 */
export function composeCommands(
  label: string,
  commands: EditCommand[],
): EditCommand {
  const ops: PrimOp[] = commands.flatMap((c) => c.ops);
  return { label, ops };
}

/** True when applying then undoing `command` restores `regions` exactly — a
 *  cheap invariant check the editor can assert in dev builds. */
export function isReversible(regions: Region[], command: EditCommand): boolean {
  const applied = applyOps(regions, command.ops);
  const back = applyOps(applied, invertOps(command.ops));
  return JSON.stringify(back) === JSON.stringify(regions);
}

/** Re-export for callers that compose timeline-aware edits. */
export { regionDurationMs };
