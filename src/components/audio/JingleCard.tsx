import { Play, Plus } from "lucide-react";

import { cn } from "@/lib/cn";
import { formatDuration } from "@/lib/format";
import { Waveform } from "./Waveform";

/**
 * A card in the jingle gallery (Phase 6). Shows the jingle's name, a mini
 * waveform, duration, and an optional variant count. Two affordances: play a
 * preview, and drop it into a project's intro/outro slot.
 */
export function JingleCard({
  name,
  durationMs,
  peaks,
  variants = 1,
  onPlay,
  onUse,
  className,
}: {
  name: string;
  durationMs: number;
  peaks: number[];
  /** Number of saved variants (longer/shorter, alternate voice). */
  variants?: number;
  onPlay?: () => void;
  onUse?: () => void;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "group flex flex-col gap-2 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-3 transition-colors hover:border-[var(--color-accent)]",
        className,
      )}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="truncate text-ui-sm font-medium">{name}</div>
          <div className="font-mono text-[11px] text-[var(--color-fg-muted)]">
            {formatDuration(durationMs)}
            {variants > 1 && ` · ${variants} variants`}
          </div>
        </div>
        <button
          onClick={onPlay}
          aria-label={`Play ${name}`}
          className="grid size-8 shrink-0 place-items-center rounded-full bg-[var(--color-accent)] text-[var(--color-accent-fg)] transition-transform hover:scale-105"
        >
          <Play
            size={14}
            fill="currentColor"
            strokeWidth={0}
            className="ml-0.5"
          />
        </button>
      </div>

      <Waveform peaks={peaks} variant="mini" />

      <button
        onClick={onUse}
        className="flex items-center justify-center gap-1.5 rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] py-1.5 text-ui-xs font-medium text-[var(--color-fg-muted)] transition-colors hover:text-[var(--color-fg)]"
      >
        <Plus size={13} />
        Use in project
      </button>
    </div>
  );
}
