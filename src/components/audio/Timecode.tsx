import { useEffect, useRef, useState } from "react";

import { cn } from "@/lib/cn";
import { formatTimecode, parseTimecode } from "@/lib/format";

/**
 * Monospaced `HH:MM:SS.mmm` timecode. Display-only by default; pass `onCommit`
 * to make it editable — click to edit, Enter/blur commits a parsed value,
 * Escape reverts. Invalid input is rejected (the previous value is kept), so
 * the transport can never be driven to a nonsense position.
 */
export function Timecode({
  ms,
  onCommit,
  size = "md",
  className,
}: {
  ms: number;
  onCommit?: (ms: number) => void;
  size?: "sm" | "md" | "lg";
  className?: string;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (editing) inputRef.current?.select();
  }, [editing]);

  const sizeClass = {
    sm: "text-ui-sm",
    md: "text-ui-lg",
    lg: "text-ui-3xl",
  }[size];

  function start() {
    if (!onCommit) return;
    setDraft(formatTimecode(ms));
    setEditing(true);
  }

  function commit() {
    const parsed = parseTimecode(draft);
    if (parsed !== null && onCommit) onCommit(parsed);
    setEditing(false);
  }

  if (editing) {
    return (
      <input
        ref={inputRef}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") commit();
          if (e.key === "Escape") setEditing(false);
        }}
        className={cn(
          "w-[10ch] rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1 text-center font-mono tabular-nums outline-none ring-1 ring-[var(--color-accent)]",
          sizeClass,
          className,
        )}
      />
    );
  }

  return (
    <span
      onClick={start}
      role={onCommit ? "button" : undefined}
      tabIndex={onCommit ? 0 : undefined}
      onKeyDown={(e) => onCommit && e.key === "Enter" && start()}
      className={cn(
        "font-mono tabular-nums tracking-tight",
        onCommit &&
          "cursor-text rounded-[var(--radius-sm)] px-1 hover:bg-[var(--color-bg-surface)]",
        sizeClass,
        className,
      )}
    >
      {formatTimecode(ms)}
    </span>
  );
}
