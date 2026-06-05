import { cn } from "@/lib/cn";
import logoUrl from "@/assets/logo.svg";

/**
 * SundayStudio mark — the official app icon (light tile: navy mic + gold cross)
 * plus the two-tone "Sunday / Studio" wordmark.
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
      <img
        src={logoUrl}
        width={size}
        height={size}
        alt=""
        aria-hidden="true"
        className="rounded-[22%]"
        style={{ display: "block" }}
      />
      {showWordmark && (
        <span className="text-ui-lg font-semibold tracking-tight">
          Sunday<span className="text-[var(--color-accent)]">Studio</span>
        </span>
      )}
    </div>
  );
}
