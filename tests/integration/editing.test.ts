import { describe, expect, it } from "vitest";

import {
  applyOps,
  crossfadeOps,
  invertOps,
  keptSpans,
  mergeableNext,
  mergeOps,
  overlapWithPrev,
  pasteRegion,
  regionDurationMs,
  regionEndMs,
  removeSilencesOps,
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

  describe("merge", () => {
    it("is the inverse of split: a split then merge restores the original", () => {
      const r = region({
        id: "r",
        start_in_take_ms: 0,
        end_in_take_ms: 1000,
        position_in_timeline_ms: 0,
      });
      const { left, right } = splitRegion(r, 600, "right")!;
      const next = mergeableNext([left, right], left);
      expect(next?.id).toBe("right");
      const out = applyOps([left, right], mergeOps(left, next!));
      expect(out).toHaveLength(1);
      expect(out[0].id).toBe("r"); // left keeps the original region's id
      expect(out[0].start_in_take_ms).toBe(0);
      expect(out[0].end_in_take_ms).toBe(1000); // back to the full span
    });

    it("declines to merge across a gap or a different take", () => {
      const a = region({
        id: "a",
        position_in_timeline_ms: 0,
        end_in_take_ms: 1000,
      });
      const gapped = region({ id: "b", position_in_timeline_ms: 1500 });
      expect(mergeableNext([a, gapped], a)).toBeNull();
      const otherTake = region({
        id: "c",
        take_id: "t2",
        position_in_timeline_ms: 1000,
        start_in_take_ms: 0,
      });
      expect(mergeableNext([a, otherTake], a)).toBeNull();
    });
  });

  describe("crossfade", () => {
    it("detects overlap with the previous clip and fades both across it", () => {
      const prev = region({
        id: "p",
        position_in_timeline_ms: 0,
        end_in_take_ms: 1000,
      });
      const cur = region({
        id: "c",
        position_in_timeline_ms: 800,
        end_in_take_ms: 1000,
      });
      const ov = overlapWithPrev([prev, cur], cur);
      expect(ov?.overlapMs).toBe(200);
      const out = applyOps(
        [prev, cur],
        crossfadeOps(ov!.prev, cur, ov!.overlapMs),
      );
      expect(out.find((r) => r.id === "p")!.fade_out_ms).toBe(200);
      expect(out.find((r) => r.id === "c")!.fade_in_ms).toBe(200);
    });

    it("returns null when clips don't overlap", () => {
      const prev = region({
        id: "p",
        position_in_timeline_ms: 0,
        end_in_take_ms: 500,
      });
      const cur = region({ id: "c", position_in_timeline_ms: 600 });
      expect(overlapWithPrev([prev, cur], cur)).toBeNull();
    });
  });

  describe("remove silences", () => {
    let counter = 0;
    const mintId = () => `gen${counter++}`;

    it("replaces a clip with ripple-packed kept spans, dropping the silence", () => {
      counter = 0;
      const r = region({
        id: "r",
        start_in_take_ms: 0,
        end_in_take_ms: 1000,
        position_in_timeline_ms: 2000,
        fade_in_ms: 8,
        fade_out_ms: 9,
      });
      // One 200ms silence in the middle (take-time 400..600).
      const ops = removeSilencesOps(
        r,
        [{ start_ms: 400, end_ms: 600 }],
        mintId,
      );
      const out = applyOps([r], ops);
      expect(out).toHaveLength(2);
      // Kept spans: 0..400 and 600..1000.
      expect(out[0].start_in_take_ms).toBe(0);
      expect(out[0].end_in_take_ms).toBe(400);
      expect(out[0].position_in_timeline_ms).toBe(2000);
      expect(out[0].fade_in_ms).toBe(8); // original outer fade preserved
      // Second clip ripple-packed right after the first (400ms long).
      expect(out[1].start_in_take_ms).toBe(600);
      expect(out[1].position_in_timeline_ms).toBe(2400);
      expect(out[1].fade_out_ms).toBe(9);
      // Total kept duration shrank by the 200ms gap.
      expect(regionDurationMs(out[0]) + regionDurationMs(out[1])).toBe(800);
    });

    it("is a no-op when there's no silence inside the window", () => {
      const r = region({ start_in_take_ms: 0, end_in_take_ms: 1000 });
      expect(removeSilencesOps(r, [], mintId)).toEqual([]);
      // Silence entirely outside the region window is ignored too.
      expect(
        removeSilencesOps(r, [{ start_ms: 2000, end_ms: 3000 }], mintId),
      ).toEqual([]);
    });

    it("keptSpans clips silences to the region window", () => {
      const r = region({ start_in_take_ms: 100, end_in_take_ms: 900 });
      // Silence overruns both edges → clipped to [100,900] minus the middle.
      const kept = keptSpans(r, [
        { start_ms: 0, end_ms: 200 },
        { start_ms: 800, end_ms: 1200 },
      ]);
      expect(kept).toEqual([{ start_ms: 200, end_ms: 800 }]);
    });
  });

  describe("paste", () => {
    it("clones a clip with a new id at the playhead", () => {
      const src = region({
        id: "src",
        gain_adjust_db: -3,
        position_in_timeline_ms: 0,
      });
      const pasted = pasteRegion(src, "new", 5000);
      expect(pasted.id).toBe("new");
      expect(pasted.position_in_timeline_ms).toBe(5000);
      expect(pasted.gain_adjust_db).toBe(-3);
      expect(pasted.take_id).toBe(src.take_id);
    });
  });
});
