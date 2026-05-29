/**
 * Waveform peak helpers. Real peaks are precomputed from audio in Phase 3.1;
 * this module provides a deterministic synthetic generator for demos, empty
 * states, and the design system (seeded so renders are stable across runs —
 * no Math.random, which would make screenshots/goldens flaky).
 */

/** Deterministic pseudo-waveform: `count` normalised peaks (0..1), seeded. */
export function fakePeaks(count: number, seed = 1): number[] {
  let s = seed >>> 0 || 1;
  const rand = () => {
    // xorshift32 — deterministic PRNG, no Math.random.
    s ^= s << 13;
    s ^= s >>> 17;
    s ^= s << 5;
    return ((s >>> 0) % 1000) / 1000;
  };
  const out: number[] = [];
  for (let i = 0; i < count; i++) {
    const envelope = Math.sin((i / count) * Math.PI); // fade in/out
    out.push(Math.min(1, (0.25 + rand() * 0.75) * envelope));
  }
  return out;
}
