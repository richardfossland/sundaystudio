import { Circle, Headphones } from "lucide-react";

import { cn } from "@/lib/cn";
import { LevelMeter, LevelReadout } from "./LevelMeter";

export interface TrackState {
  name: string;
  /** Track accent colour (any CSS colour). */
  color: string;
  armed: boolean;
  muted: boolean;
  soloed: boolean;
  monitoring: boolean;
  /** Fader gain in dB, typically −60..+6. */
  gainDb: number;
  /** Live input level in dBFS for the strip meter. */
  levelDb: number;
  peakDb?: number;
}

/**
 * A mixer track strip header: colour tab, editable-looking name, the four state
 * toggles (arm / mute / solo / monitor), a gain fader, and a live level meter.
 * Stateless and fully controlled — the parent owns `TrackState` and handles
 * every change, so the same component serves the recorder, mixer and editor.
 */
export function TrackHeader({
  track,
  onChange,
  className,
}: {
  track: TrackState;
  onChange?: (next: Partial<TrackState>) => void;
  className?: string;
}) {
  const set = (patch: Partial<TrackState>) => onChange?.(patch);
  const dimmed = track.muted && !track.soloed;

  return (
    <div
      className={cn(
        "flex items-stretch gap-3 rounded-[var(--radius-md)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-3 transition-opacity",
        dimmed && "opacity-50",
        className,
      )}
    >
      {/* Colour tab */}
      <span
        className="w-1 shrink-0 rounded-full"
        style={{ background: track.color }}
        aria-hidden
      />

      <div className="flex min-w-0 flex-1 flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-ui-sm font-medium">{track.name}</span>
          <LevelReadout db={track.levelDb} />
        </div>

        {/* Toggles */}
        <div className="flex items-center gap-1.5">
          <Toggle
            active={track.armed}
            activeClass="bg-[var(--color-recording)] text-white"
            label="Arm for recording"
            onClick={() => set({ armed: !track.armed })}
          >
            <Circle size={11} fill="currentColor" strokeWidth={0} />
          </Toggle>
          <Toggle
            active={track.muted}
            activeClass="bg-[var(--color-fg-muted)] text-[var(--color-bg)]"
            label="Mute"
            onClick={() => set({ muted: !track.muted })}
          >
            M
          </Toggle>
          <Toggle
            active={track.soloed}
            activeClass="bg-[var(--color-accent)] text-[var(--color-accent-fg)]"
            label="Solo"
            onClick={() => set({ soloed: !track.soloed })}
          >
            S
          </Toggle>
          <Toggle
            active={track.monitoring}
            activeClass="bg-[var(--color-info)] text-white"
            label="Monitor"
            onClick={() => set({ monitoring: !track.monitoring })}
          >
            <Headphones size={12} />
          </Toggle>

          {/* Gain fader */}
          <input
            type="range"
            min={-60}
            max={6}
            step={0.5}
            value={track.gainDb}
            onChange={(e) => set({ gainDb: Number(e.target.value) })}
            aria-label="Gain"
            className="ml-1 h-1 flex-1 cursor-pointer accent-[var(--color-accent)]"
          />
          <span className="w-12 shrink-0 text-right font-mono text-[11px] tabular-nums text-[var(--color-fg-muted)]">
            {track.gainDb > 0 ? "+" : ""}
            {track.gainDb.toFixed(1)}
          </span>
        </div>
      </div>

      {/* Strip meter */}
      <LevelMeter
        db={track.levelDb}
        peakDb={track.peakDb}
        orientation="vertical"
        className="w-1.5"
      />
    </div>
  );
}

function Toggle({
  active,
  activeClass,
  label,
  onClick,
  children,
}: {
  active: boolean;
  activeClass: string;
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      aria-label={label}
      aria-pressed={active}
      title={label}
      className={cn(
        "grid size-6 place-items-center rounded-[var(--radius-sm)] text-[11px] font-bold transition-colors",
        active
          ? activeClass
          : "bg-[var(--color-bg-surface)] text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]",
      )}
    >
      {children}
    </button>
  );
}
