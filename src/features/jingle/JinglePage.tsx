/**
 * JinglePage — the polished surface for the headline jingle feature.
 *
 * The generation panel (reused `JingleSpecForm`) sits up top; everything it
 * produces drops into a gallery of cards below. Each card can be previewed
 * (a plain `<audio>` element streaming the generated `audio_url`), renamed,
 * regenerated from its original spec, or deleted.
 *
 * Generation is one jingle at a time through the `ai_jingle_generate` command
 * (Sunday Cast Pro). The backend has no "list jingles" command — each result
 * comes back from a generate call — so the gallery is owned here, in the
 * renderer, keyed by a stable client id. Off-Pro / offline, generation throws
 * a clean validation error which the form surfaces inline; the gallery simply
 * stays empty until the first success.
 */

import { useRef, useState } from "react";
import {
  ArrowLeft,
  Check,
  Loader2,
  Music,
  Pause,
  Pencil,
  Play,
  RefreshCw,
  Trash2,
} from "lucide-react";

import { Brand } from "@/components/Brand";
import { Button } from "@/components/ui/Button";
import { ipc } from "@/lib/ipc";
import { useI18n } from "@/lib/i18n";
import type { JingleResult } from "@/lib/bindings";
import type { JingleSpec } from "@/lib/jingle";

import { JingleSpecForm } from "./JingleSpecForm";

/** One entry in the gallery: a generated result plus the spec that made it. */
export interface GalleryJingle {
  /** Stable client-side id (the backend doesn't persist jingles yet). */
  id: string;
  result: JingleResult;
  spec: JingleSpec;
}

let _seq = 0;
/** Monotonic client id — stable across renders, unique per generated jingle. */
function nextId(): string {
  _seq += 1;
  return `jingle-${Date.now().toString(36)}-${_seq}`;
}

export function JinglePage({ onBack }: { onBack?: () => void }) {
  const { t } = useI18n();
  const [jingles, setJingles] = useState<GalleryJingle[]>([]);

  function handleGenerated(result: JingleResult, spec: JingleSpec) {
    setJingles((prev) => [{ id: nextId(), result, spec }, ...prev]);
  }

  function rename(id: string, title: string) {
    setJingles((prev) =>
      prev.map((j) =>
        j.id === id ? { ...j, result: { ...j.result, title } } : j,
      ),
    );
  }

  function replace(id: string, result: JingleResult) {
    setJingles((prev) => prev.map((j) => (j.id === id ? { ...j, result } : j)));
  }

  function remove(id: string) {
    setJingles((prev) => prev.filter((j) => j.id !== id));
  }

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl px-6 py-10">
        <header className="mb-8 flex items-center justify-between">
          <Brand size={32} />
          <div className="flex items-center gap-2">
            <span className="rounded-full border border-[var(--color-border)] px-2.5 py-1 text-ui-xs font-medium uppercase tracking-widest text-[var(--color-accent)]">
              {t("navJingle")}
            </span>
            {onBack && (
              <Button variant="ghost" size="sm" onClick={onBack}>
                <ArrowLeft size={15} />
                {t("navBack")}
              </Button>
            )}
          </div>
        </header>

        {/* Generation panel — reused form, reporting up into the gallery. */}
        <JingleSpecForm onGenerated={handleGenerated} />

        {/* Gallery */}
        <section className="mt-10">
          <div className="mb-4 flex items-baseline justify-between">
            <h2 className="flex items-center gap-2 text-ui-md font-semibold">
              <Music size={16} className="text-[var(--color-accent)]" />
              {t("jinglePageGalleryTitle")}
            </h2>
            {jingles.length > 0 && (
              <span className="text-ui-xs text-[var(--color-fg-muted)]">
                {t("jinglePageCount").replace("{n}", String(jingles.length))}
              </span>
            )}
          </div>

          {jingles.length === 0 ? (
            <p className="rounded-[var(--radius-lg)] border border-dashed border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-6 text-center text-ui-sm text-[var(--color-fg-muted)]">
              {t("jinglePageGalleryEmpty")}
            </p>
          ) : (
            <ul className="grid gap-3 sm:grid-cols-2">
              {jingles.map((j) => (
                <JingleCard
                  key={j.id}
                  jingle={j}
                  onRename={(title) => rename(j.id, title)}
                  onRegenerated={(result) => replace(j.id, result)}
                  onDelete={() => remove(j.id)}
                />
              ))}
            </ul>
          )}
        </section>
      </div>
    </div>
  );
}

// ── Card ────────────────────────────────────────────────────────────────────

function JingleCard({
  jingle,
  onRename,
  onRegenerated,
  onDelete,
}: {
  jingle: GalleryJingle;
  onRename: (title: string) => void;
  onRegenerated: (result: JingleResult) => void;
  onDelete: () => void;
}) {
  const { t } = useI18n();
  const audioRef = useRef<HTMLAudioElement | null>(null);

  const [playing, setPlaying] = useState(false);
  const [editing, setEditing] = useState(false);
  const [draftTitle, setDraftTitle] = useState(jingle.result.title);
  const [regenerating, setRegenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function togglePlay() {
    const el = audioRef.current;
    if (!el) return;
    if (playing) {
      el.pause();
    } else {
      // `play()` may reject offline / when the URL can't stream; surface it.
      void el.play().catch((e) => {
        setError(e instanceof Error ? e.message : String(e));
        setPlaying(false);
      });
    }
  }

  function commitRename() {
    const next = draftTitle.trim();
    if (next && next !== jingle.result.title) onRename(next);
    setEditing(false);
  }

  async function regenerate() {
    setRegenerating(true);
    setError(null);
    try {
      const result = await ipc.ai.generateJingle(jingle.spec);
      onRegenerated(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRegenerating(false);
    }
  }

  return (
    <li className="flex flex-col gap-3 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-4">
      {/* Title row */}
      <div className="flex items-start justify-between gap-2">
        {editing ? (
          <input
            type="text"
            value={draftTitle}
            autoFocus
            onChange={(e) => setDraftTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") commitRename();
              if (e.key === "Escape") {
                setDraftTitle(jingle.result.title);
                setEditing(false);
              }
            }}
            placeholder={t("jinglePageRenamePlaceholder")}
            aria-label={t("jinglePageRename")}
            className="min-w-0 flex-1 rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-2 py-1 text-ui-sm outline-none focus:border-[var(--color-accent)]"
          />
        ) : (
          <h3 className="min-w-0 flex-1 truncate text-ui-md font-semibold">
            {jingle.result.title}
          </h3>
        )}
        {editing ? (
          <Button
            variant="ghost"
            size="sm"
            onClick={commitRename}
            aria-label={t("actionSave")}
          >
            <Check size={14} />
          </Button>
        ) : (
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              setDraftTitle(jingle.result.title);
              setEditing(true);
            }}
            aria-label={t("jinglePageRename")}
          >
            <Pencil size={14} />
          </Button>
        )}
      </div>

      {/* Metadata */}
      <div className="flex flex-wrap gap-1.5 text-[11px] text-[var(--color-fg-muted)]">
        <span className="rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1.5 py-0.5 font-medium">
          {jingle.result.model}
        </span>
        <span className="rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1.5 py-0.5">
          {jingle.result.duration_sec}s
        </span>
        <span className="rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1.5 py-0.5">
          {jingle.spec.tempo_bpm} BPM
        </span>
        <span className="rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-1.5 py-0.5">
          {t(`jingleMood${capitalize(jingle.spec.mood)}`)}
        </span>
      </div>

      {/* Hidden audio element — the preview transport. */}
      <audio
        ref={audioRef}
        src={jingle.result.audio_url}
        preload="none"
        onPlay={() => setPlaying(true)}
        onPause={() => setPlaying(false)}
        onEnded={() => setPlaying(false)}
        data-testid="jingle-audio"
      />

      {/* Actions */}
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="accent"
          size="sm"
          onClick={togglePlay}
          aria-label={playing ? t("jinglePagePause") : t("jinglePagePlay")}
        >
          {playing ? <Pause size={14} /> : <Play size={14} />}
          {playing ? t("jinglePagePause") : t("jinglePagePlay")}
        </Button>
        <Button
          variant="surface"
          size="sm"
          onClick={regenerate}
          disabled={regenerating}
        >
          {regenerating ? (
            <Loader2 size={14} className="animate-spin" />
          ) : (
            <RefreshCw size={14} />
          )}
          {regenerating
            ? t("jinglePageRegenerating")
            : t("jinglePageRegenerate")}
        </Button>
        <Button
          variant="ghost"
          size="sm"
          onClick={onDelete}
          aria-label={`${t("jinglePageDelete")} ${jingle.result.title}`}
        >
          <Trash2 size={14} />
        </Button>
      </div>

      {error && (
        <p role="alert" className="text-ui-xs text-[var(--color-danger)]">
          {error}
        </p>
      )}
    </li>
  );
}

function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
