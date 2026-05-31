import { describe, expect, it } from "vitest";

import {
  apply,
  canRedo,
  canUndo,
  composeCommands,
  deleteCommand,
  duplicateCommand,
  emptyStack,
  isReversible,
  MAX_GAIN_DB,
  MIN_GAIN_DB,
  MIN_REGION_MS,
  nudgeCommand,
  redo,
  setGainCommand,
  trimEndCommand,
  trimStartCommand,
  undo,
} from "@/lib/editStack";
import { CUT_FADE_MS, regionEndMs } from "@/lib/editing";
import type { Region } from "@/lib/bindings";

function region(over: Partial<Region> = {}): Region {
  return {
    id: "r1",
    take_id: "t1",
    source_track_id: "s1",
    target_track_id: "trk1",
    start_in_take_ms: 0,
    end_in_take_ms: 1000,
    position_in_timeline_ms: 0,
    fade_in_ms: 5,
    fade_out_ms: 5,
    gain_adjust_db: 0,
    ...over,
  };
}

describe("edit stack: undo/redo invariants", () => {
  it("apply pushes to past and clears the redo branch", () => {
    const r = region();
    let regions = [r];
    let stack = emptyStack();
    ({ regions, stack } = apply(stack, regions, setGainCommand(r, -6)));
    expect(stack.past).toHaveLength(1);
    expect(stack.future).toHaveLength(0);
    expect(regions[0].gain_adjust_db).toBe(-6);
  });

  it("undo then redo round-trips the regions exactly", () => {
    const r = region();
    const start = [r];
    let regions = start;
    let stack = emptyStack();
    ({ regions, stack } = apply(stack, regions, setGainCommand(r, -6)));
    const afterApply = regions;

    ({ regions, stack } = undo(stack, regions));
    expect(regions).toEqual(start);
    expect(canRedo(stack)).toBe(true);
    expect(canUndo(stack)).toBe(false);

    ({ regions, stack } = redo(stack, regions));
    expect(regions).toEqual(afterApply);
    expect(canUndo(stack)).toBe(true);
    expect(canRedo(stack)).toBe(false);
  });

  it("a new apply after an undo forks history (drops the redo branch)", () => {
    const r = region();
    let regions = [r];
    let stack = emptyStack();
    ({ regions, stack } = apply(stack, regions, setGainCommand(r, -6)));
    ({ regions, stack } = undo(stack, regions));
    expect(canRedo(stack)).toBe(true);
    // A fresh edit on the restored region must clear the redo branch.
    ({ regions, stack } = apply(stack, regions, setGainCommand(regions[0], 3)));
    expect(canRedo(stack)).toBe(false);
    expect(stack.past).toHaveLength(1);
    expect(regions[0].gain_adjust_db).toBe(3);
  });

  it("undo and redo are no-ops at the ends of history", () => {
    const regions = [region()];
    const stack = emptyStack();
    expect(undo(stack, regions)).toEqual({ regions, stack });
    expect(redo(stack, regions)).toEqual({ regions, stack });
  });

  it("a no-op command does not pollute the undo stack", () => {
    const r = region({ gain_adjust_db: -6 });
    const stack = emptyStack();
    const out = apply(stack, [r], setGainCommand(r, -6)); // same value
    expect(out.stack.past).toHaveLength(0);
    expect(out.regions).toEqual([r]);
  });

  it("caps depth, dropping the oldest command", () => {
    let regions = [region({ gain_adjust_db: 0 })];
    let stack = emptyStack(2);
    for (const g of [-1, -2, -3]) {
      ({ regions, stack } = apply(
        stack,
        regions,
        setGainCommand(regions[0], g),
      ));
    }
    // Only the last 2 commands are retained; current gain is the latest.
    expect(stack.past).toHaveLength(2);
    expect(regions[0].gain_adjust_db).toBe(-3);
    // The dropped first command can no longer be undone past two steps.
    ({ regions, stack } = undo(stack, regions));
    ({ regions, stack } = undo(stack, regions));
    expect(canUndo(stack)).toBe(false);
    expect(regions[0].gain_adjust_db).toBe(-1); // not back to 0 (oldest dropped)
  });
});

describe("edit stack: command builders", () => {
  it("nudge clamps to a non-negative timeline position", () => {
    const r = region({ position_in_timeline_ms: 500 });
    expect(nudgeCommand(r, 250).ops).toHaveLength(1);
    // Past zero clamps to 0.
    const out = nudgeCommand(r, -800);
    const after = (out.ops[0] as { after: Region }).after;
    expect(after.position_in_timeline_ms).toBe(0);
    // Exactly at the clamp boundary with no movement is a no-op.
    expect(
      nudgeCommand(region({ position_in_timeline_ms: 0 }), -10).ops,
    ).toEqual([]);
  });

  it("set gain clamps to the allowed dB window", () => {
    const r = region({ gain_adjust_db: 0 });
    const loud = setGainCommand(r, 999);
    expect((loud.ops[0] as { after: Region }).after.gain_adjust_db).toBe(
      MAX_GAIN_DB,
    );
    const quiet = setGainCommand(r, -999);
    expect((quiet.ops[0] as { after: Region }).after.gain_adjust_db).toBe(
      MIN_GAIN_DB,
    );
  });

  it("trim start moves both timeline pos and take start in lockstep", () => {
    const r = region({
      start_in_take_ms: 300,
      end_in_take_ms: 1300, // 1000ms clip
      position_in_timeline_ms: 1000,
    });
    // Trim the left edge 200ms later (to timeline 1200).
    const cmd = trimStartCommand(r, 1200);
    const after = (cmd.ops[0] as { after: Region }).after;
    expect(after.position_in_timeline_ms).toBe(1200);
    expect(after.start_in_take_ms).toBe(500); // 300 + 200
    expect(after.end_in_take_ms).toBe(1300); // unchanged
    expect(after.fade_in_ms).toBe(CUT_FADE_MS);
  });

  it("trim start cannot expose take before the take head (clamps left)", () => {
    const r = region({
      start_in_take_ms: 100, // only 100ms of head available
      end_in_take_ms: 1100,
      position_in_timeline_ms: 1000,
    });
    // Ask to drag the left edge 500ms earlier; only 100ms of head exists.
    const cmd = trimStartCommand(r, 500);
    const after = (cmd.ops[0] as { after: Region }).after;
    expect(after.position_in_timeline_ms).toBe(900); // 1000 - 100
    expect(after.start_in_take_ms).toBe(0);
  });

  it("trim start clamps so the clip keeps at least MIN_REGION_MS", () => {
    const r = region({
      start_in_take_ms: 0,
      end_in_take_ms: 1000,
      position_in_timeline_ms: 0,
    });
    const cmd = trimStartCommand(r, 5000); // way past the right edge
    const after = (cmd.ops[0] as { after: Region }).after;
    expect(after.position_in_timeline_ms).toBe(regionEndMs(r) - MIN_REGION_MS);
  });

  it("trim end shortens the take window with a cut fade", () => {
    const r = region({
      start_in_take_ms: 0,
      end_in_take_ms: 1000,
      position_in_timeline_ms: 2000,
    });
    const cmd = trimEndCommand(r, 2600); // 600ms long now
    const after = (cmd.ops[0] as { after: Region }).after;
    expect(after.end_in_take_ms).toBe(600);
    expect(after.fade_out_ms).toBe(CUT_FADE_MS);
  });

  it("trim end is bounded by the take length when known", () => {
    const r = region({
      start_in_take_ms: 200,
      end_in_take_ms: 1000,
      position_in_timeline_ms: 0,
    });
    // Take is only 1500ms total; from start_in_take 200 there's 1300ms of tail.
    const cmd = trimEndCommand(r, 99999, 1500);
    const after = (cmd.ops[0] as { after: Region }).after;
    expect(after.end_in_take_ms).toBe(1500); // capped at the take length
  });

  it("trim end keeps at least MIN_REGION_MS", () => {
    const r = region({ position_in_timeline_ms: 1000, end_in_take_ms: 1000 });
    const cmd = trimEndCommand(r, 0); // before the clip start
    const after = (cmd.ops[0] as { after: Region }).after;
    // newEnd clamps to position + MIN; take end = start + MIN.
    expect(after.end_in_take_ms).toBe(r.start_in_take_ms + MIN_REGION_MS);
  });

  it("duplicate places a copy right after the original", () => {
    const r = region({ position_in_timeline_ms: 1000, end_in_take_ms: 1000 });
    const cmd = duplicateCommand(r, "dup");
    const created = (cmd.ops[0] as { region: Region }).region;
    expect(created.id).toBe("dup");
    expect(created.position_in_timeline_ms).toBe(regionEndMs(r));
    expect(cmd.ops[0].kind).toBe("create");
  });
});

describe("edit stack: compose + reversibility", () => {
  it("composes commands into one undoable unit, dropping no-ops", () => {
    const r = region({ id: "a", position_in_timeline_ms: 500 });
    const batch = composeCommands("Batch", [
      nudgeCommand(r, 100),
      setGainCommand(r, -3),
      nudgeCommand(region({ position_in_timeline_ms: 0 }), -10), // no-op
    ]);
    expect(batch.ops).toHaveLength(2);
  });

  it("a composed batch is fully reversible by undo", () => {
    const r = region({ id: "a", position_in_timeline_ms: 500 });
    const start = [r];
    const batch = composeCommands("Batch", [
      nudgeCommand(r, 100),
      setGainCommand({ ...r, position_in_timeline_ms: 600 }, -3),
    ]);
    let regions = start;
    let stack = emptyStack();
    ({ regions, stack } = apply(stack, regions, batch));
    expect(regions[0].position_in_timeline_ms).toBe(600);
    expect(regions[0].gain_adjust_db).toBe(-3);
    ({ regions } = undo(stack, regions));
    expect(regions).toEqual(start);
  });

  it("isReversible confirms each builder round-trips", () => {
    const r = region({
      start_in_take_ms: 200,
      end_in_take_ms: 1200,
      position_in_timeline_ms: 1000,
    });
    expect(isReversible([r], nudgeCommand(r, 250))).toBe(true);
    expect(isReversible([r], setGainCommand(r, -6))).toBe(true);
    expect(isReversible([r], trimStartCommand(r, 1100))).toBe(true);
    expect(isReversible([r], trimEndCommand(r, 1800))).toBe(true);
    expect(isReversible([r], duplicateCommand(r, "x"))).toBe(true);
    expect(isReversible([r], deleteCommand(r))).toBe(true);
  });
});
