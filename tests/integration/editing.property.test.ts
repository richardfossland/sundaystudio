import { describe, expect, it } from "vitest";

import {
  applyOps,
  crossfadeOps,
  invertOps,
  mergeableNext,
  mergeOps,
  overlapWithPrev,
  regionDurationMs,
  removeSilencesOps,
  rippleDeleteOps,
  splitRegion,
  type PrimOp,
  type Span,
} from "@/lib/editing";
import type { Region } from "@/lib/bindings";

/**
 * Deterministic property/invariant fuzz of the edit command algebra. These pin
 * round-trip and length invariants that the example-based suite only checks for
 * single hand-picked cases. Fixed-seed PRNG so failures are reproducible; small
 * iteration caps so the run stays cheap.
 */

// --- deterministic PRNG (mulberry32) -------------------------------------
function rng(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a |= 0;
    a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const randInt = (r: () => number, lo: number, hi: number) =>
  lo + Math.floor(r() * (hi - lo + 1));

function makeRegion(id: string, over: Partial<Region> = {}): Region {
  return {
    id,
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

/** Canonical comparison of region state independent of array ordering. */
function sortById(rs: Region[]): Region[] {
  return [...rs].sort((a, b) => a.id.localeCompare(b.id));
}

describe("editing command algebra — property fuzz", () => {
  it("apply ∘ apply(invert) is the identity for any command (random ops)", () => {
    const r = rng(0xc0ffee);
    for (let iter = 0; iter < 500; iter++) {
      // A random starting timeline of 1..5 clips on one track.
      const n = randInt(r, 1, 5);
      const start: Region[] = [];
      let pos = 0;
      for (let i = 0; i < n; i++) {
        const dur = randInt(r, 100, 1000);
        start.push(
          makeRegion(`r${i}`, {
            position_in_timeline_ms: pos,
            start_in_take_ms: 0,
            end_in_take_ms: dur,
          }),
        );
        pos += dur + randInt(r, 0, 300);
      }

      // Build a random command from one of the algebra's op generators.
      const target = start[randInt(r, 0, start.length - 1)];
      let ops: PrimOp[];
      switch (randInt(r, 0, 3)) {
        case 0: {
          // split at a random interior playhead (skip if not splittable)
          const s = target.position_in_timeline_ms;
          const e = s + regionDurationMs(target);
          const play = randInt(r, s + 1, e - 1);
          const res = splitRegion(target, play, "split-new");
          ops =
            res == null
              ? []
              : [
                  { kind: "update", before: target, after: res.left },
                  { kind: "create", region: res.right },
                ];
          break;
        }
        case 1:
          ops = rippleDeleteOps(start, target);
          break;
        case 2: {
          let c = 0;
          ops = removeSilencesOps(
            target,
            [
              {
                start_ms: target.start_in_take_ms + 50,
                end_ms: target.start_in_take_ms + 100,
              },
            ],
            () => `gen${c++}`,
          );
          break;
        }
        default:
          ops = [
            {
              kind: "update",
              before: target,
              after: { ...target, gain_adjust_db: randInt(r, -24, 24) },
            },
          ];
      }

      const after = applyOps(start, ops);
      const back = applyOps(after, invertOps(ops));
      expect(sortById(back)).toEqual(sortById(start));
    }
  });

  it("split then merge restores the original region for any interior playhead", () => {
    const r = rng(0x5eed);
    for (let iter = 0; iter < 500; iter++) {
      const dur = randInt(r, 4, 4000);
      const orig = makeRegion("orig", {
        start_in_take_ms: randInt(r, 0, 500),
        end_in_take_ms: 0,
        position_in_timeline_ms: randInt(r, 0, 5000),
        fade_in_ms: randInt(r, 0, 20),
        fade_out_ms: randInt(r, 0, 20),
      });
      orig.end_in_take_ms = orig.start_in_take_ms + dur;

      const s = orig.position_in_timeline_ms;
      const e = s + dur;
      const play = randInt(r, s + 1, e - 1);
      const res = splitRegion(orig, play, "right");
      if (res == null) continue; // not splittable at this playhead
      const { left, right } = res;

      const timeline = [left, right];
      const next = mergeableNext(timeline, left);
      expect(next).not.toBeNull();
      const merged = applyOps(timeline, mergeOps(left, next!));

      expect(merged).toHaveLength(1);
      const m = merged[0];
      expect(m.id).toBe(orig.id);
      expect(m.start_in_take_ms).toBe(orig.start_in_take_ms);
      expect(m.end_in_take_ms).toBe(orig.end_in_take_ms);
      expect(m.position_in_timeline_ms).toBe(orig.position_in_timeline_ms);
      // No audio span lost across the round trip.
      expect(regionDurationMs(m)).toBe(dur);
    }
  });

  it("ripple delete conserves later-clip durations and never moves a clip below 0", () => {
    const r = rng(0xd00d);
    for (let iter = 0; iter < 500; iter++) {
      const n = randInt(r, 2, 6);
      const clips: Region[] = [];
      let pos = 0;
      for (let i = 0; i < n; i++) {
        const dur = randInt(r, 50, 800);
        clips.push(
          makeRegion(`r${i}`, {
            position_in_timeline_ms: pos,
            start_in_take_ms: 0,
            end_in_take_ms: dur,
          }),
        );
        pos += dur + randInt(r, 0, 200);
      }
      const target = clips[randInt(r, 0, clips.length - 1)];
      const gap = regionDurationMs(target);

      const out = applyOps(clips, rippleDeleteOps(clips, target));
      // Target gone, every other clip survives with its duration intact.
      expect(out.some((c) => c.id === target.id)).toBe(false);
      expect(out).toHaveLength(clips.length - 1);
      for (const before of clips) {
        if (before.id === target.id) continue;
        const a = out.find((c) => c.id === before.id)!;
        expect(regionDurationMs(a)).toBe(regionDurationMs(before));
        expect(a.position_in_timeline_ms).toBeGreaterThanOrEqual(0);
        if (before.position_in_timeline_ms >= target.position_in_timeline_ms) {
          // Pulled earlier by exactly the gap (clamped at 0).
          expect(a.position_in_timeline_ms).toBe(
            Math.max(0, before.position_in_timeline_ms - gap),
          );
        } else {
          expect(a.position_in_timeline_ms).toBe(
            before.position_in_timeline_ms,
          );
        }
      }
    }
  });

  it("removeSilences conserves total kept duration and packs without gaps", () => {
    const r = rng(0xa11ce);
    for (let iter = 0; iter < 500; iter++) {
      const len = randInt(r, 200, 4000);
      const region = makeRegion("r", {
        start_in_take_ms: 0,
        end_in_take_ms: len,
        position_in_timeline_ms: randInt(r, 0, 2000),
      });
      // A few non-overlapping silent spans inside the window.
      const silences: Span[] = [];
      let cur = randInt(r, 0, 100);
      while (cur < len) {
        const gapStart = cur + randInt(r, 20, 200);
        const gapEnd = gapStart + randInt(r, 10, 150);
        if (gapEnd >= len) break;
        silences.push({ start_ms: gapStart, end_ms: gapEnd });
        cur = gapEnd + randInt(r, 20, 200);
      }
      let c = 0;
      const ops = removeSilencesOps(region, silences, () => `g${c++}`);
      if (ops.length === 0) continue; // nothing to remove

      const out = applyOps([region], ops);
      const totalSilence = silences.reduce(
        (acc, s) => acc + (s.end_ms - s.start_ms),
        0,
      );
      const keptTotal = out.reduce((acc, k) => acc + regionDurationMs(k), 0);
      // Total kept = original minus the removed silence.
      expect(keptTotal).toBe(len - totalSilence);
      // Clips are packed contiguously from the region position with no gaps.
      const sorted = [...out].sort(
        (a, b) => a.position_in_timeline_ms - b.position_in_timeline_ms,
      );
      let expectedPos = region.position_in_timeline_ms;
      for (const clip of sorted) {
        expect(clip.position_in_timeline_ms).toBe(expectedPos);
        expectedPos += regionDurationMs(clip);
      }
    }
  });

  it("crossfade ops never produce a fade longer than the clip and apply cleanly", () => {
    const r = rng(0xbeef);
    for (let iter = 0; iter < 500; iter++) {
      const prevDur = randInt(r, 100, 1000);
      const prev = makeRegion("p", {
        position_in_timeline_ms: 0,
        start_in_take_ms: 0,
        end_in_take_ms: prevDur,
      });
      const overlap = randInt(r, 1, prevDur);
      const cur = makeRegion("c", {
        position_in_timeline_ms: prevDur - overlap,
        start_in_take_ms: 0,
        end_in_take_ms: randInt(r, overlap, 1000),
      });
      const ov = overlapWithPrev([prev, cur], cur);
      expect(ov).not.toBeNull();
      expect(ov!.overlapMs).toBe(overlap);
      const out = applyOps(
        [prev, cur],
        crossfadeOps(ov!.prev, cur, ov!.overlapMs),
      );
      const p = out.find((x) => x.id === "p")!;
      const c = out.find((x) => x.id === "c")!;
      // The crossfade equals the overlap and fits inside both clip windows.
      expect(p.fade_out_ms).toBe(overlap);
      expect(c.fade_in_ms).toBe(overlap);
      expect(p.fade_out_ms).toBeLessThanOrEqual(regionDurationMs(p));
      expect(c.fade_in_ms).toBeLessThanOrEqual(regionDurationMs(c));
    }
  });
});
