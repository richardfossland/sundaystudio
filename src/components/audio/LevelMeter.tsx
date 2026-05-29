import { cn } from "@/lib/cn";
import {
  dbToFraction,
  meterZone,
  zoneColorVar,
  METER_FLOOR_DB,
} from "@/lib/meter";

/**
 * A broadcast-style level meter. Renders a dBFS level as a filled bar whose
 * colour follows the green/yellow/red zones, with an optional peak-hold tick.
 *
 * Orientation defaults to vertical (mixer strips); horizontal suits inline
 * transport readouts. The fill is a CSS gradient so a single element covers all
 * three zones — cheap enough to update at 60fps from the meter atomics later.
 */
export function LevelMeter({
  db,
  peakDb,
  orientation = "vertical",
  className,
}: {
  /** Current level in dBFS (0 = full scale). */
  db: number;
  /** Optional peak-hold level in dBFS, drawn as a thin tick. */
  peakDb?: number;
  orientation?: "vertical" | "horizontal";
  className?: string;
}) {
  const fill = dbToFraction(db);
  const peakFill = peakDb !== undefined ? dbToFraction(peakDb) : undefined;
  const vertical = orientation === "vertical";

  // A static three-stop gradient mirroring the zone thresholds (-60..0 dBFS).
  // green up to -12 (0.8 of the bar), yellow to -3 (0.95), red to top.
  const gradient = `linear-gradient(${vertical ? "to top" : "to right"},
    var(--color-meter-green) 0%,
    var(--color-meter-green) 80%,
    var(--color-meter-yellow) 80%,
    var(--color-meter-yellow) 95%,
    var(--color-meter-red) 95%)`;

  return (
    <div
      role="meter"
      aria-valuemin={METER_FLOOR_DB}
      aria-valuemax={0}
      aria-valuenow={Math.round(db)}
      className={cn(
        "relative overflow-hidden rounded-full bg-[var(--color-neutral-900)]",
        vertical ? "h-full w-2" : "h-2 w-full",
        className,
      )}
    >
      {/* Fill, masked to the current level via inset on the long axis. */}
      <div
        className="absolute inset-0 transition-[clip-path] duration-75"
        style={{
          backgroundImage: gradient,
          clipPath: vertical
            ? `inset(${(1 - fill) * 100}% 0 0 0)`
            : `inset(0 ${(1 - fill) * 100}% 0 0)`,
        }}
      />
      {/* Peak-hold tick */}
      {peakFill !== undefined && (
        <div
          className="absolute bg-[var(--color-fg)]"
          style={
            vertical
              ? {
                  left: 0,
                  right: 0,
                  bottom: `calc(${peakFill * 100}% - 1px)`,
                  height: 2,
                }
              : {
                  top: 0,
                  bottom: 0,
                  left: `calc(${peakFill * 100}% - 1px)`,
                  width: 2,
                }
          }
        />
      )}
    </div>
  );
}

/** The numeric dBFS readout that often sits beside a meter. */
export function LevelReadout({ db }: { db: number }) {
  const zone = meterZone(db);
  return (
    <span
      className="font-mono text-[11px] tabular-nums"
      style={{ color: zoneColorVar(zone) }}
    >
      {db <= METER_FLOOR_DB ? "-∞" : `${db.toFixed(1)}`} dB
    </span>
  );
}
