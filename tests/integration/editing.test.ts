import { describe, expect, it } from "vitest";

import {
  applyOps,
  invertOps,
  regionDurationMs,
  regionEndMs,
  rippleDeleteOps,
  splitRegion,
  type PrimOp,
} from "@/lib/editing";
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

describe("editing command algebra", () => {
  it("inverts ops as a reverse, swapped list", () => {
    const a = region({ id: "a" });
    const b1 = region({ id: "b", gain_adjust_db: 0 });
    const b2 = region({ id: "b", gain_adjust_db: -3 });
    const ops: PrimOp[] = [
      { kind: "create", region: a },
      { kind: "update", before: b1, after: b2 },
    ];
    const inv = invertOps(ops);
    expect(inv[0]).toEqual({ kind: "update", before: b2, after: b1 });
    expect(inv[1]).toEqual({ kind: "delete", region: a });
  });

  it("apply then apply-inverse round-trips the state", () => {
    const start = [region({ id: "a" })];
    const newR = region({ id: "b", position_in_timeline_ms: 2000 });
    const ops: PrimOp[] = [{ kind: "create", region: newR }];
    const after = applyOps(start, ops);
    expect(after).toHaveLength(2);
    const back = applyOps(after, invertOps(ops));
    expect(back).toEqual(start);
  });

  it("applies update by id", () => {
    const before = region({ id: "a", gain_adjust_db: 0 });
    const after = region({ id: "a", gain_adjust_db: -6 });
    const out = applyOps([before], [{ kind: "update", before, after }]);
    expect(out[0].gain_adjust_db).toBe(-6);
  });

  it("computes duration and end on the timeline", () => {
    const r = region({
      start_in_take_ms: 500,
      end_in_take_ms: 2000,
      position_in_timeline_ms: 1000,
    });
    expect(regionDurationMs(r)).toBe(1500);
    expect(regionEndMs(r)).toBe(2500);
  });

  describe("split", () => {
    it("splits at an interior playhead, mapping timeline → take offset", () => {
      const r = region({
        start_in_take_ms: 200,
        end_in_take_ms: 1200, // 1000ms long
        position_in_timeline_ms: 3000,
      });
      const res = splitRegion(r, 3400, "new"); // 400ms into the clip
      expect(res).not.toBeNull();
      const { left, right } = res!;
      // Left keeps its start, ends at the cut (take offset 200+400=600).
      expect(left.start_in_take_ms).toBe(200);
      expect(left.end_in_take_ms).toBe(600);
      expect(left.fade_out_ms).toBe(5);
      // Right starts at the cut, keeps the original end, sits at the playhead.
      expect(right.id).toBe("new");
      expect(right.start_in_take_ms).toBe(600);
      expect(right.end_in_take_ms).toBe(1200);
      expect(right.position_in_timeline_ms).toBe(3400);
      expect(right.fade_in_ms).toBe(5);
      // No audio lost: the two halves cover the original span.
      expect(regionDurationMs(left) + regionDurationMs(right)).toBe(
        regionDurationMs(r),
      );
    });

    it("returns null when the playhead is on or outside an edge", () => {
      const r = region({ position_in_timeline_ms: 1000, end_in_take_ms: 1000 });
      expect(splitRegion(r, 1000, "x")).toBeNull(); // left edge
      expect(splitRegion(r, 2000, "x")).toBeNull(); // right edge
      expect(splitRegion(r, 500, "x")).toBeNull(); // before
    });
  });

  describe("ripple delete", () => {
    it("removes the target and pulls later same-track clips earlier", () => {
      const a = region({
        id: "a",
        position_in_timeline_ms: 0,
        end_in_take_ms: 1000,
      });
      const b = region({
        id: "b",
        position_in_timeline_ms: 1000,
        end_in_take_ms: 1000,
      });
      const c = region({
        id: "c",
        position_in_timeline_ms: 2000,
        end_in_take_ms: 1000,
      });
      const ops = rippleDeleteOps([a, b, c], b);
      const out = applyOps([a, b, c], ops);
      expect(out.map((r) => r.id).sort()).toEqual(["a", "c"]);
      // c shifts left by b's 1000ms duration; a is untouched.
      expect(out.find((r) => r.id === "c")!.position_in_timeline_ms).toBe(1000);
      expect(out.find((r) => r.id === "a")!.position_in_timeline_ms).toBe(0);
    });

    it("ignores clips on other tracks", () => {
      const a = region({
        id: "a",
        position_in_timeline_ms: 0,
        end_in_take_ms: 500,
      });
      const other = region({
        id: "o",
        target_track_id: "trk2",
        position_in_timeline_ms: 1000,
      });
      const ops = rippleDeleteOps([a, other], a);
      // Only the delete of `a` — no shift op for the other-track clip.
      expect(ops).toHaveLength(1);
      expect(ops[0]).toEqual({ kind: "delete", region: a });
    });
  });
});
