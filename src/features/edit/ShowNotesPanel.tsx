import { useState } from "react";
import {
  Check,
  Copy,
  KeyRound,
  Loader2,
  Plus,
  Sparkles,
  Trash2,
} from "lucide-react";

import { Button } from "@/components/ui/Button";
import { StatusBanner } from "@/components/ui/StatusBanner";
import { errorMessage, IPCError, ipc } from "@/lib/ipc";
import { useI18n } from "@/lib/i18n";
import type { ShowNotes, ShowNotesChapter } from "@/lib/bindings";

/** A chapter the user is editing in the panel (AI-suggested or hand-added). */
type EditableChapter = { id: string; startMs: number; title: string };

/**
 * The AI show-notes panel for the editor (Phase 5.2, Sunday Cast Pro).
 *
 * Takes the episode's transcript — pre-seeded from a SundayRec deep-link handoff
 * when present, otherwise pasted — and asks Claude for title options, a Norwegian
 * + English summary, timestamped chapters, tags and a few highlight-clip
 * suggestions. The model only suggests; the backend sanitizes every field, and
 * the user curates the chapters here before they're embedded on export.
 *
 * KEYLESS FALLBACK: with no `ANTHROPIC_API_KEY` the backend returns a clean
 * `validation` error; we surface "legg til nøkkel for AI" and the manual chapter
 * editor stays fully usable — chapters export without any key. The feature never
 * blocks the core edit/export flow.
 */
export function ShowNotesPanel({
  durationMs,
  initialTranscript,
  initialContext,
  initialGlossary,
  fromHandoff,
  onChaptersChange,
}: {
  /** Total programme length in ms, so timestamps clamp into the real take. */
  durationMs: number;
  /** Transcript seeded from a handoff, if any. */
  initialTranscript?: string;
  /** Context primer carried from the deep link (e.g. "Sermon, speaker: Ola"). */
  initialContext?: string;
  /** Glossary terms carried from the deep link. */
  initialGlossary?: string[];
  /** True when the transcript arrived via a SundayRec handoff (shows a note). */
  fromHandoff?: boolean;
  /** Lifts the curated chapters up so the export call can embed them. */
  onChaptersChange?: (chapters: ShowNotesChapter[]) => void;
}) {
  const t = useI18n((s) => s.t);

  const [transcript, setTranscript] = useState(initialTranscript ?? "");
  const [generating, setGenerating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [needsKey, setNeedsKey] = useState(false);
  const [notes, setNotes] = useState<ShowNotes | null>(null);
  const [chapters, setChapters] = useState<EditableChapter[]>([]);
  const [chaptersApplied, setChaptersApplied] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  function publishChapters(next: EditableChapter[]) {
    setChapters(next);
    onChaptersChange?.(
      next
        .filter((c) => c.title.trim().length > 0)
        .map((c) => ({ start_ms: c.startMs, title: c.title.trim() })),
    );
  }

  async function generate() {
    setError(null);
    setNeedsKey(false);
    if (transcript.trim().length === 0) {
      setError(t("showNotesNeedTranscript"));
      return;
    }
    setGenerating(true);
    try {
      const result = await ipc.ai.showNotes({
        transcript,
        duration_ms: durationMs,
        context: initialContext ?? null,
        glossary: initialGlossary ?? [],
      });
      setNotes(result);
      // Seed the editable chapter list from the AI suggestion.
      const seeded: EditableChapter[] = result.chapters.map((c) => ({
        id: crypto.randomUUID(),
        startMs: c.start_ms,
        title: c.title,
      }));
      publishChapters(seeded);
      setChaptersApplied(false);
    } catch (e) {
      // A missing/invalid key surfaces as a `validation` IPCError — show the
      // keyless prompt rather than a scary error, and keep manual chapters live.
      if (e instanceof IPCError && e.code === "validation") {
        setNeedsKey(true);
      } else {
        setError(errorMessage(e));
      }
    } finally {
      setGenerating(false);
    }
  }

  function addChapter() {
    publishChapters([
      ...chapters,
      { id: crypto.randomUUID(), startMs: 0, title: "" },
    ]);
  }

  function updateChapter(id: string, patch: Partial<EditableChapter>) {
    publishChapters(
      chapters.map((c) => (c.id === id ? { ...c, ...patch } : c)),
    );
  }

  function removeChapter(id: string) {
    publishChapters(chapters.filter((c) => c.id !== id));
  }

  function applyChapters() {
    // Chapters are already lifted on every edit; this just confirms intent and
    // shows the user they'll be embedded on export.
    onChaptersChange?.(
      chapters
        .filter((c) => c.title.trim().length > 0)
        .map((c) => ({ start_ms: c.startMs, title: c.title.trim() })),
    );
    setChaptersApplied(true);
  }

  async function copy(key: string, text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied((c) => (c === key ? null : c)), 1500);
    } catch {
      // Clipboard may be unavailable; ignore — copy is a convenience.
    }
  }

  return (
    <div className="flex flex-col gap-4 p-5">
      <div>
        <div className="flex items-center gap-2">
          <Sparkles size={16} className="text-[var(--color-accent)]" />
          <h2 className="text-ui-md font-semibold">{t("showNotesTitle")}</h2>
        </div>
        <p className="mt-1 text-ui-sm text-[var(--color-fg-muted)]">
          {t("showNotesDesc")}
        </p>
      </div>

      {/* Transcript source */}
      <label className="flex flex-col gap-1.5">
        <span className="text-ui-xs font-medium text-[var(--color-fg-muted)]">
          {t("showNotesTranscriptLabel")}
        </span>
        {fromHandoff && (
          <span className="text-[11px] text-[var(--color-accent)]">
            {t("showNotesTranscriptFromHandoff")}
          </span>
        )}
        <textarea
          value={transcript}
          onChange={(e) => setTranscript(e.target.value)}
          placeholder={t("showNotesTranscriptPlaceholder")}
          rows={6}
          className="resize-y rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-2 text-ui-sm"
        />
      </label>

      <div className="flex items-center gap-3">
        <Button
          variant="accent"
          size="sm"
          onClick={generate}
          disabled={generating}
        >
          {generating ? (
            <Loader2 size={15} className="animate-spin" />
          ) : (
            <Sparkles size={15} />
          )}
          {generating ? t("showNotesGenerating") : t("showNotesGenerate")}
        </Button>
        <span className="text-[11px] text-[var(--color-fg-muted)]">
          {t("showNotesManualHint")}
        </span>
      </div>

      {needsKey && (
        <div className="flex items-start gap-2 rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] px-3 py-2.5">
          <KeyRound
            size={15}
            className="mt-0.5 shrink-0 text-[var(--color-accent)]"
          />
          <div>
            <div className="text-ui-sm font-medium">{t("showNotesNoKey")}</div>
            <div className="text-[11px] text-[var(--color-fg-muted)]">
              {t("showNotesNoKeyHint")}
            </div>
          </div>
        </div>
      )}

      {error && (
        <StatusBanner kind="danger" testId="shownotes-error" message={error} />
      )}

      {notes && (
        <div className="flex flex-col gap-4">
          <div className="text-[11px] text-[var(--color-fg-muted)]">
            {t("showNotesModelLabel")} {notes.model}
          </div>

          {notes.title_options.length > 0 && (
            <Section title={t("showNotesSectionTitles")}>
              <ul className="flex flex-col gap-1">
                {notes.title_options.map((title, i) => (
                  <li
                    key={i}
                    className="flex items-center justify-between gap-2 text-ui-sm"
                  >
                    <span>{title}</span>
                    <CopyButton
                      label={
                        copied === `title-${i}`
                          ? t("showNotesCopied")
                          : t("showNotesCopy")
                      }
                      done={copied === `title-${i}`}
                      onClick={() => copy(`title-${i}`, title)}
                    />
                  </li>
                ))}
              </ul>
            </Section>
          )}

          {notes.summary_no && (
            <Section title={t("showNotesSectionSummaryNo")}>
              <SummaryBlock
                text={notes.summary_no}
                done={copied === "sum-no"}
                copyLabel={
                  copied === "sum-no"
                    ? t("showNotesCopied")
                    : t("showNotesCopy")
                }
                onCopy={() => copy("sum-no", notes.summary_no)}
              />
            </Section>
          )}

          {notes.summary_en && (
            <Section title={t("showNotesSectionSummaryEn")}>
              <SummaryBlock
                text={notes.summary_en}
                done={copied === "sum-en"}
                copyLabel={
                  copied === "sum-en"
                    ? t("showNotesCopied")
                    : t("showNotesCopy")
                }
                onCopy={() => copy("sum-en", notes.summary_en)}
              />
            </Section>
          )}

          {notes.tags.length > 0 && (
            <Section title={t("showNotesSectionTags")}>
              <div className="flex flex-wrap gap-1.5">
                {notes.tags.map((tag, i) => (
                  <span
                    key={i}
                    className="rounded-full border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-2.5 py-0.5 text-[11px]"
                  >
                    {tag}
                  </span>
                ))}
              </div>
            </Section>
          )}

          {notes.clips.length > 0 && (
            <Section title={t("showNotesSectionClips")}>
              <ul className="flex flex-col gap-1.5">
                {notes.clips.map((clip, i) => (
                  <li key={i} className="flex items-baseline gap-2 text-ui-sm">
                    <span className="shrink-0 font-mono tabular-nums text-[var(--color-fg-muted)]">
                      {formatMs(clip.start_ms)}–{formatMs(clip.end_ms)}
                    </span>
                    <span>{clip.reason}</span>
                  </li>
                ))}
              </ul>
            </Section>
          )}
        </div>
      )}

      {/* Chapters editor — always available, with or without AI. */}
      <Section title={t("showNotesSectionChapters")}>
        {chapters.length === 0 ? (
          <p className="text-ui-sm text-[var(--color-fg-muted)]">
            {t("showNotesNoChapters")}
          </p>
        ) : (
          <ul className="flex flex-col gap-1.5">
            {chapters.map((c) => (
              <li key={c.id} className="flex items-center gap-2">
                <input
                  type="text"
                  inputMode="numeric"
                  value={formatMs(c.startMs)}
                  onChange={(e) =>
                    updateChapter(c.id, { startMs: parseMs(e.target.value) })
                  }
                  className="w-20 rounded-[var(--radius-xs)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-2 py-1 text-center font-mono text-[11px] tabular-nums"
                  aria-label="Chapter start"
                />
                <input
                  type="text"
                  value={c.title}
                  onChange={(e) =>
                    updateChapter(c.id, { title: e.target.value })
                  }
                  placeholder={t("showNotesChapterTitlePlaceholder")}
                  className="flex-1 rounded-[var(--radius-xs)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-2 py-1 text-ui-sm"
                />
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={() => removeChapter(c.id)}
                  aria-label="Remove chapter"
                >
                  <Trash2 size={14} />
                </Button>
              </li>
            ))}
          </ul>
        )}
        <div className="mt-2 flex items-center gap-2">
          <Button variant="surface" size="sm" onClick={addChapter}>
            <Plus size={14} />
            {t("showNotesAddChapter")}
          </Button>
          {chapters.some((c) => c.title.trim().length > 0) && (
            <Button variant="ghost" size="sm" onClick={applyChapters}>
              <Check size={14} />
              {t("showNotesUseChapters")}
            </Button>
          )}
        </div>
        {chaptersApplied && (
          <p className="mt-2 text-[11px] text-[var(--color-accent)]">
            {t("showNotesChaptersApplied")}
          </p>
        )}
      </Section>
    </div>
  );
}

function Section({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-[var(--radius-sm)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-3">
      <h3 className="mb-2 text-ui-xs font-semibold uppercase tracking-wide text-[var(--color-fg-muted)]">
        {title}
      </h3>
      {children}
    </section>
  );
}

function SummaryBlock({
  text,
  copyLabel,
  done,
  onCopy,
}: {
  text: string;
  copyLabel: string;
  done: boolean;
  onCopy: () => void;
}) {
  return (
    <div className="flex items-start justify-between gap-3">
      <p className="text-ui-sm leading-relaxed">{text}</p>
      <CopyButton label={copyLabel} done={done} onClick={onCopy} />
    </div>
  );
}

function CopyButton({
  label,
  done,
  onClick,
}: {
  label: string;
  done: boolean;
  onClick: () => void;
}) {
  return (
    <Button variant="ghost" size="sm" onClick={onClick} className="shrink-0">
      {done ? <Check size={13} /> : <Copy size={13} />}
      {label}
    </Button>
  );
}

/** Format ms as `M:SS` (or `H:MM:SS`) for the chapter / clip displays. */
function formatMs(ms: number): string {
  const total = Math.max(0, Math.round(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

/** Parse `H:MM:SS` / `M:SS` / `SS` back into ms (best-effort, never throws). */
function parseMs(value: string): number {
  const parts = value.split(":").map((p) => Number(p.trim()) || 0);
  let secs = 0;
  for (const part of parts) secs = secs * 60 + part;
  return Math.max(0, Math.round(secs * 1000));
}
