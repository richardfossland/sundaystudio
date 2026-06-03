import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { save, open as openDialog } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  AlertTriangle,
  FolderOpen,
  Mic,
  Music,
  Palette,
  Settings,
} from "lucide-react";

import { Brand } from "@/components/Brand";
import { Button } from "@/components/ui/Button";
import { ipc } from "@/lib/ipc";
import { useSession } from "@/lib/session";
import type { TemplateInfo } from "@/lib/bindings";

/**
 * The Start screen (Phase 2.3): pick a quick-start template to create a new
 * project, reopen a recent one, or open an existing `.scast` folder. This is
 * the app's entry point when no project is open.
 *
 * Templates and the recent list come from the backend; project creation/open
 * use the native file dialog. Outside the Tauri runtime (e.g. a browser) the
 * lists are empty and the dialogs are unavailable — the screen degrades to its
 * chrome.
 */
export function StartPage({
  onOpenSettings,
  onOpenDesign,
  onOpenDiagnostics,
  onOpenJingle,
}: {
  onOpenSettings?: () => void;
  onOpenDesign?: () => void;
  onOpenDiagnostics?: () => void;
  onOpenJingle?: () => void;
}) {
  const setSnapshot = useSession((s) => s.setSnapshot);
  const templates = useQuery({
    queryKey: ["project_templates"],
    queryFn: ipc.project.templates,
  });
  const recent = useQuery({
    queryKey: ["project_recent"],
    queryFn: ipc.project.recent,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function fail(err: unknown) {
    setError(err instanceof Error ? err.message : String(err));
    setBusy(false);
  }

  async function createFrom(template: TemplateInfo) {
    setError(null);
    try {
      const picked = await save({
        title: "Create project",
        defaultPath: `${template.label}.scast`,
      });
      if (!picked) return;
      const path = picked.endsWith(".scast") ? picked : `${picked}.scast`;
      setBusy(true);
      const snapshot = await ipc.project.createFromTemplate(
        path,
        projectName(path),
        template.id,
      );
      setSnapshot(snapshot);
    } catch (err) {
      fail(err);
    }
  }

  async function openExisting(path?: string) {
    setError(null);
    try {
      const dir =
        path ?? (await openDialog({ directory: true, title: "Open project" }));
      if (!dir || Array.isArray(dir)) return;
      setBusy(true);
      setSnapshot(await ipc.project.open(dir));
    } catch (err) {
      fail(err);
    }
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl px-8 py-10">
        <header className="mb-10 flex items-center justify-between">
          <Brand size={32} />
          <div className="flex items-center gap-2">
            <Button variant="surface" size="sm" onClick={() => openExisting()}>
              <FolderOpen size={15} />
              Open project
            </Button>
            {onOpenJingle && (
              <Button
                variant="accent"
                size="sm"
                onClick={onOpenJingle}
                aria-label="Jingle Studio"
              >
                <Music size={15} />
                Jingle
              </Button>
            )}
            {onOpenDesign && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onOpenDesign}
                aria-label="Design system"
              >
                <Palette size={15} />
              </Button>
            )}
            {onOpenSettings && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onOpenSettings}
                aria-label="Audio settings"
              >
                <Settings size={15} />
              </Button>
            )}
            {onOpenDiagnostics && (
              <Button
                variant="ghost"
                size="sm"
                onClick={onOpenDiagnostics}
                aria-label="Diagnostics"
              >
                <Activity size={15} />
              </Button>
            )}
          </div>
        </header>

        {error && (
          <div className="mb-6 flex items-start gap-2 rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3 text-ui-sm text-[var(--color-danger)]">
            <AlertTriangle size={16} className="mt-0.5 shrink-0" />
            <span className="break-all">{error}</span>
          </div>
        )}

        <h1 className="mb-1 text-ui-2xl font-bold">Start a new podcast</h1>
        <p className="mb-6 text-ui-sm text-[var(--color-fg-muted)]">
          Pick a template — tracks, colours and inputs are pre-configured. You
          can change everything later.
        </p>

        {templates.isSuccess ? (
          <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-3">
            {templates.data.map((t) => (
              <TemplateCard
                key={t.id}
                template={t}
                disabled={busy}
                onPick={() => createFrom(t)}
              />
            ))}
          </div>
        ) : (
          <p className="text-ui-sm text-[var(--color-fg-muted)]">
            {templates.isError
              ? "Templates load when running in the SundayStudio app."
              : "Loading templates…"}
          </p>
        )}

        {recent.isSuccess && recent.data.length > 0 && (
          <section className="mt-10">
            <h2 className="mb-3 text-ui-xs font-semibold uppercase tracking-wider text-[var(--color-fg-muted)]">
              Recent
            </h2>
            <ul className="flex flex-col gap-1.5">
              {recent.data.map((p) => (
                <li key={p.path}>
                  <button
                    onClick={() => openExisting(p.path)}
                    disabled={busy}
                    className="flex w-full items-baseline justify-between gap-3 rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-3 py-2 text-left transition-colors hover:bg-[var(--color-neutral-700)] disabled:opacity-50"
                  >
                    <span className="truncate text-ui-sm font-medium">
                      {p.name}
                    </span>
                    <span className="shrink-0 truncate font-mono text-[11px] text-[var(--color-fg-muted)]">
                      {p.path}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
    </div>
  );
}

function TemplateCard({
  template,
  disabled,
  onPick,
}: {
  template: TemplateInfo;
  disabled: boolean;
  onPick: () => void;
}) {
  return (
    <button
      onClick={onPick}
      disabled={disabled}
      className="group flex flex-col gap-3 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-4 text-left transition-colors hover:border-[var(--color-accent)] disabled:opacity-50"
    >
      <div className="flex items-start justify-between gap-2">
        <h3 className="text-ui-md font-semibold">{template.label}</h3>
        <span className="flex shrink-0 items-center gap-1 rounded-full bg-[var(--color-bg-surface)] px-2 py-0.5 text-[11px] font-medium text-[var(--color-fg-muted)]">
          {template.mic_count > 0 ? (
            <>
              <Mic size={11} />
              {template.mic_count}
            </>
          ) : (
            "blank"
          )}
        </span>
      </div>
      <p className="min-h-8 text-ui-xs text-[var(--color-fg-muted)]">
        {template.description}
      </p>
      <div className="flex flex-wrap gap-1">
        {template.tracks.map((tr, i) => (
          <span
            key={`${tr.name}-${i}`}
            className="flex items-center gap-1 rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1.5 py-0.5 text-[10px]"
          >
            <span
              className="size-2 rounded-full"
              style={{ background: tr.color }}
            />
            {tr.input_assignment === null ? (
              <Music size={9} className="text-[var(--color-fg-muted)]" />
            ) : null}
            {tr.name}
          </span>
        ))}
      </div>
    </button>
  );
}

/** Derive a project name from a chosen `.scast` path (basename, no extension). */
function projectName(path: string): string {
  const base = path.split(/[/\\]/).pop() ?? path;
  return base.replace(/\.scast$/i, "") || "Untitled";
}
