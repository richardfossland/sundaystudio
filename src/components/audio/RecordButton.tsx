import { Circle, Square } from "lucide-react";

import { cn } from "@/lib/cn";

export type RecordState = "idle" | "armed" | "recording";

/**
 * The transport's record control. Three states with distinct affordances:
 *   - idle      gold-ringed disc, inviting
 *   - armed     gold ring breathing, "ready" — a click starts recording
 *   - recording saturated red, pulsing, with a stop glyph
 *
 * Animations are CSS-driven (defined inline via Tailwind utilities) so they run
 * off the main thread and never compete with the audio engine.
 */
export function RecordButton({
  state = "idle",
  size = 72,
  onClick,
  disabled = false,
  className,
}: {
  state?: RecordState;
  size?: number;
  onClick?: () => void;
  /** Disable while a start/stop transition is in flight. */
  disabled?: boolean;
  className?: string;
}) {
  const recording = state === "recording";
  const armed = state === "armed";

  return (
    <button
      onClick={onClick}
      disabled={disabled}
      aria-label={recording ? "Stop recording" : "Record"}
      aria-pressed={recording}
      className={cn(
        "relative grid place-items-center rounded-full transition-[transform,box-shadow] duration-[var(--duration-fast)] active:scale-95 disabled:cursor-not-allowed disabled:opacity-60",
        className,
      )}
      style={{ width: size, height: size }}
    >
      {/* Outer ring */}
      <span
        className={cn(
          "absolute inset-0 rounded-full border-2",
          recording
            ? "border-[var(--color-recording)]"
            : "border-[var(--color-accent)]",
          armed && "animate-ping opacity-60",
        )}
      />
      {/* Inner fill */}
      <span
        className={cn(
          "grid place-items-center rounded-full transition-colors",
          recording
            ? "bg-[var(--color-recording)] text-white animate-pulse"
            : "bg-[var(--color-accent)] text-[var(--color-accent-fg)]",
        )}
        style={{ width: size - 16, height: size - 16 }}
      >
        {recording ? (
          <Square size={size * 0.28} fill="currentColor" strokeWidth={0} />
        ) : (
          <Circle size={size * 0.34} fill="currentColor" strokeWidth={0} />
        )}
      </span>
    </button>
  );
}
