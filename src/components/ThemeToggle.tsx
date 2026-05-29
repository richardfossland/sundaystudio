import { Monitor, Moon, Sun } from "lucide-react";

import { cn } from "@/lib/cn";
import { useTheme, type ThemeMode } from "@/lib/theme";

const MODES: { mode: ThemeMode; icon: typeof Sun; label: string }[] = [
  { mode: "system", icon: Monitor, label: "System" },
  { mode: "light", icon: Sun, label: "Light" },
  { mode: "dark", icon: Moon, label: "Dark" },
];

/** Three-way theme switch (system / light / dark). */
export function ThemeToggle() {
  const { mode, setMode } = useTheme();
  return (
    <div className="inline-flex rounded-[var(--radius-md)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-0.5">
      {MODES.map(({ mode: m, icon: Icon, label }) => (
        <button
          key={m}
          onClick={() => setMode(m)}
          aria-label={label}
          aria-pressed={mode === m}
          className={cn(
            "grid size-7 place-items-center rounded-[var(--radius-sm)] transition-colors",
            mode === m
              ? "bg-[var(--color-bg-surface)] text-[var(--color-fg)]"
              : "text-[var(--color-fg-muted)] hover:text-[var(--color-fg)]",
          )}
        >
          <Icon size={14} />
        </button>
      ))}
    </div>
  );
}
