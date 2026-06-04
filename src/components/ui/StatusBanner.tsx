import {
  AlertTriangle,
  CheckCircle2,
  Info,
  OctagonAlert,
  type LucideIcon,
} from "lucide-react";

import { cn } from "@/lib/cn";

/**
 * StatusBanner — the inline status/error strip used across the feature pages.
 *
 * This consolidates the copy-pasted "border-b bar with an icon and a message"
 * markup that grew independently in RecordingPage and EditPage. Keeping it in
 * one place means every invoke error is surfaced the same way (same colours,
 * same icon, same `role`), instead of each page caring for itself — or, as in
 * RecordingPage's track/marker mutations, not surfacing the failure at all.
 *
 * Variants map to the Sunday token palette:
 *  - `danger`  → recoverable error (a failed mutation, a bad import)
 *  - `critical`→ loud, full-bleed alert (recording is sacred; e.g. writer loss)
 *  - `success` → a completed action
 *  - `info`    → a neutral note
 */
export type StatusKind = "danger" | "critical" | "success" | "info";

const ICON: Record<StatusKind, LucideIcon> = {
  danger: AlertTriangle,
  critical: OctagonAlert,
  success: CheckCircle2,
  info: Info,
};

const TONE: Record<StatusKind, string> = {
  danger:
    "bg-[var(--color-bg-surface)] text-[var(--color-danger)] border-[var(--color-border)]",
  critical:
    "bg-[var(--color-danger)] text-white border-[var(--color-danger)] font-semibold",
  success:
    "bg-[var(--color-bg-surface)] text-[var(--color-success)] border-[var(--color-border)]",
  info: "bg-[var(--color-bg-surface)] text-[var(--color-fg-muted)] border-[var(--color-border)]",
};

export function StatusBanner({
  kind,
  message,
  testId,
  className,
}: {
  kind: StatusKind;
  message: React.ReactNode;
  testId?: string;
  className?: string;
}) {
  const Icon = ICON[kind];
  // `critical` is an assertive alert; the rest are polite status updates.
  const role = kind === "critical" || kind === "danger" ? "alert" : "status";
  return (
    <div
      role={role}
      data-testid={testId}
      className={cn(
        "flex items-start gap-2 border-b px-5 py-2 text-ui-xs",
        TONE[kind],
        className,
      )}
    >
      <Icon size={14} className="mt-0.5 shrink-0" />
      <span className="break-all">{message}</span>
    </div>
  );
}
