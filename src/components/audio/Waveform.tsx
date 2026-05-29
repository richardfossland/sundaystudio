import { useEffect, useRef } from "react";

import { cn } from "@/lib/cn";

/**
 * Canvas waveform renderer. Draws a normalised peak array (values 0..1) as a
 * mirrored fill around the centre line. Canvas (not SVG) because real projects
 * have hundreds of regions and must stay at 60fps — this component is the seed
 * of the Phase 3.1 timeline renderer, which adds zoom levels and peak caching.
 *
 * Two presets: `mini` (compact thumbnail, e.g. a JingleCard) and `full`
 * (editor lane height). Colour follows the audio palette.
 */
export function Waveform({
  peaks,
  variant = "full",
  color = "var(--color-waveform)",
  progress,
  className,
}: {
  /** Normalised peak magnitudes, 0..1, one per horizontal bucket. */
  peaks: number[];
  variant?: "mini" | "full";
  /** Optional 0..1 playback progress; the played portion uses the peak colour. */
  progress?: number;
  color?: string;
  className?: string;
}) {
  const ref = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = ref.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const { clientWidth: w, clientHeight: h } = canvas;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);
    ctx.clearRect(0, 0, w, h);

    if (peaks.length === 0) return;
    const mid = h / 2;
    const barWidth = w / peaks.length;
    const gap = variant === "mini" ? 0 : Math.min(1, barWidth * 0.25);
    const playedUntil = progress !== undefined ? progress * w : -1;

    for (let i = 0; i < peaks.length; i++) {
      const x = i * barWidth;
      const amp = Math.max(0.02, peaks[i]) * (mid - 1);
      ctx.fillStyle = x <= playedUntil ? "var(--color-waveform-peak)" : color;
      // CSS variables don't resolve in canvas; read computed values once.
      ctx.fillStyle = resolveColor(canvas, ctx.fillStyle);
      ctx.fillRect(x, mid - amp, Math.max(1, barWidth - gap), amp * 2);
    }
  }, [peaks, variant, color, progress]);

  return (
    <canvas
      ref={ref}
      className={cn(
        "block w-full",
        variant === "mini" ? "h-8" : "h-20",
        className,
      )}
    />
  );
}

/** Resolve a `var(--x)` token to a concrete colour via a throwaway element. */
function resolveColor(el: HTMLElement, value: string): string {
  if (!value.startsWith("var(")) return value;
  const name = value.slice(4, -1).trim();
  return getComputedStyle(el).getPropertyValue(name).trim() || "#5b8def";
}
