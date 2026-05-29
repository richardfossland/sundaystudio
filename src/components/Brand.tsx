import { cn } from "@/lib/cn";

/**
 * SundayStudio mark — a Sunday-gold disc with three rising sound waves, plus
 * the two-tone "Sunday / Studio" wordmark. A placeholder until the final logo
 * asset lands in Phase 0.3; intentionally simple, drawn in code so it scales.
 */
export function Brand({
  size = 28,
  showWordmark = true,
  className,
}: {
  size?: number;
  showWordmark?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("flex items-center gap-2.5", className)}>
      <svg
        width={size}
        height={size}
        viewBox="0 0 32 32"
        fill="none"
        aria-hidden="true"
      >
        <circle cx="16" cy="16" r="16" fill="var(--color-gold-400)" />
        {/* three sound waves of increasing height, centred */}
        <g
          stroke="var(--color-sunday-blue-950)"
          strokeWidth="2.4"
          strokeLinecap="round"
        >
          <line x1="11" y1="12.5" x2="11" y2="19.5" />
          <line x1="16" y1="9" x2="16" y2="23" />
          <line x1="21" y1="13.5" x2="21" y2="18.5" />
        </g>
      </svg>
      {showWordmark && (
        <span className="text-ui-lg font-semibold tracking-tight">
          Sunday<span className="text-[var(--color-accent)]">Studio</span>
        </span>
      )}
    </div>
  );
}
