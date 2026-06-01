/**
 * JingleSpecForm — offline jingle spec editor.
 *
 * Collects all fields for a `JingleSpec`, validates them inline, and (once
 * submitted) shows the computed `JingleRenderPlan` so the user can see exactly
 * what ffmpeg will do before any Suno call is made.
 *
 * The "Generate jingle" button is intentionally disabled with a Pro notice —
 * the form's job is spec authoring + plan preview, which is free.
 */

import { useState } from "react";
import { ChevronDown, Music, Sparkles, Wand2 } from "lucide-react";

import { Button } from "@/components/ui/Button";
import { useI18n } from "@/lib/i18n";
import {
  jingle_render_plan,
  validateJingleSpec,
  VALID_DURATIONS,
  VALID_MOODS,
  MIN_BPM,
  MAX_BPM,
  type JingleSpec,
  type JingleRenderPlan,
  type JingleDuration,
  type JingleMood,
} from "@/lib/jingle";

const DEFAULT_SPEC: JingleSpec = {
  title: "",
  duration_sec: 30,
  mood: "professional",
  tempo_bpm: 120,
  instruments: ["piano", "strings"],
  voiceover_text: "",
};

export function JingleSpecForm() {
  const { t } = useI18n();

  const [spec, setSpec] = useState<JingleSpec>(DEFAULT_SPEC);
  const [instrRaw, setInstrRaw] = useState(DEFAULT_SPEC.instruments.join(", "));
  const [plan, setPlan] = useState<JingleRenderPlan | null>(null);
  const [showPlan, setShowPlan] = useState(false);
  const [submitted, setSubmitted] = useState(false);

  // Derive instruments from raw string on every change
  function updateInstrRaw(raw: string) {
    setInstrRaw(raw);
    const parsed = raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    setSpec((s) => ({ ...s, instruments: parsed }));
  }

  const validation = validateJingleSpec(spec);
  const errors = validation.ok
    ? {}
    : Object.fromEntries(
        (
          validation as {
            ok: false;
            errors: { field: string; message: string }[];
          }
        ).errors.map((e) => [e.field, e.message]),
      );

  function handlePreview() {
    setSubmitted(true);
    if (!validation.ok) return;
    const computed = jingle_render_plan(spec);
    setPlan(computed);
    setShowPlan(true);
  }

  const patch = <K extends keyof JingleSpec>(k: K, v: JingleSpec[K]) =>
    setSpec((s) => ({ ...s, [k]: v }));

  const fieldErr = (k: string) =>
    submitted && errors[k] ? (
      <p className="mt-1 text-ui-xs text-[var(--color-danger)]">{errors[k]}</p>
    ) : null;

  return (
    <div className="mx-auto max-w-2xl px-6 py-10">
      <header className="mb-8">
        <div className="mb-1 flex items-center gap-2 text-ui-xs font-medium uppercase tracking-widest text-[var(--color-accent)]">
          <Music size={14} />
          {t("jingleTitle")}
        </div>
        <h1 className="text-ui-2xl font-bold">{t("jingleFormTitle")}</h1>
        <p className="mt-1 text-ui-sm text-[var(--color-fg-muted)]">
          {t("jingleFormDesc")}
        </p>
      </header>

      <div className="space-y-5 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        {/* Title */}
        <Field label={t("jingleFieldTitle")}>
          <input
            type="text"
            value={spec.title}
            onChange={(e) => patch("title", e.target.value)}
            placeholder={t("jingleFieldTitlePlaceholder")}
            className={inputClass(submitted && !!errors.title)}
            aria-invalid={submitted && !!errors.title}
          />
          {fieldErr("title")}
        </Field>

        {/* Duration + Mood */}
        <div className="grid grid-cols-2 gap-4">
          <Field label={t("jingleFieldDuration")}>
            <div className="relative">
              <select
                value={spec.duration_sec}
                onChange={(e) =>
                  patch(
                    "duration_sec",
                    Number(e.target.value) as JingleDuration,
                  )
                }
                className={selectClass(false)}
              >
                {VALID_DURATIONS.map((d) => (
                  <option key={d} value={d}>
                    {t(`jingleDuration${d}`)}
                  </option>
                ))}
              </select>
              <ChevronDown
                size={14}
                className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--color-fg-muted)]"
              />
            </div>
          </Field>

          <Field label={t("jingleFieldMood")}>
            <div className="relative">
              <select
                value={spec.mood}
                onChange={(e) => patch("mood", e.target.value as JingleMood)}
                className={selectClass(submitted && !!errors.mood)}
                aria-invalid={submitted && !!errors.mood}
              >
                {VALID_MOODS.map((m) => (
                  <option key={m} value={m}>
                    {t(`jingleMood${capitalize(m)}`)}
                  </option>
                ))}
              </select>
              <ChevronDown
                size={14}
                className="pointer-events-none absolute right-3 top-1/2 -translate-y-1/2 text-[var(--color-fg-muted)]"
              />
            </div>
            {fieldErr("mood")}
          </Field>
        </div>

        {/* Tempo */}
        <Field label={t("jingleFieldTempo")}>
          <input
            type="number"
            min={MIN_BPM}
            max={MAX_BPM}
            step={1}
            value={spec.tempo_bpm}
            onChange={(e) => patch("tempo_bpm", Number(e.target.value))}
            className={inputClass(submitted && !!errors.tempo_bpm)}
            aria-invalid={submitted && !!errors.tempo_bpm}
          />
          {fieldErr("tempo_bpm")}
        </Field>

        {/* Instruments */}
        <Field
          label={t("jingleFieldInstruments")}
          hint={t("jingleFieldInstrumentsHint")}
        >
          <input
            type="text"
            value={instrRaw}
            onChange={(e) => updateInstrRaw(e.target.value)}
            placeholder={t("jingleFieldInstrumentsPlaceholder")}
            className={inputClass(
              submitted &&
                !!(errors.instruments || errors["instruments.length"]),
            )}
            aria-invalid={
              submitted &&
              !!(errors.instruments || errors["instruments.length"])
            }
          />
          {fieldErr("instruments")}
          {fieldErr("instruments.length")}
        </Field>

        {/* Voiceover */}
        <Field
          label={t("jingleFieldVoiceover")}
          hint={t("jingleFieldVoiceoverHint")}
        >
          <textarea
            value={spec.voiceover_text ?? ""}
            onChange={(e) => patch("voiceover_text", e.target.value)}
            placeholder={t("jingleFieldVoiceoverPlaceholder")}
            rows={3}
            className={`${inputClass(false)} resize-y`}
          />
        </Field>
      </div>

      {/* Actions */}
      <div className="mt-5 flex flex-wrap items-center gap-3">
        <Button variant="surface" onClick={handlePreview}>
          <Wand2 size={15} />
          {t("jinglePreviewPlan")}
        </Button>
        <Button variant="accent" disabled title={t("jingleProNotice")}>
          <Sparkles size={15} />
          {t("jingleGenerateButton")}
        </Button>
        <span className="text-ui-xs text-[var(--color-fg-muted)]">
          {t("jingleProNotice")}
        </span>
      </div>

      {/* Render plan display */}
      {plan && showPlan && (
        <section className="mt-8 space-y-4 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
          <h2 className="text-ui-md font-semibold">{t("jinglePlanTitle")}</h2>

          <dl className="space-y-2 text-ui-sm">
            <PlanRow label={t("jinglePlanDescription")}>
              {plan.description}
            </PlanRow>
            <PlanRow label={t("jinglePlanOutput")}>
              <code className="font-mono text-ui-xs text-[var(--color-fg-muted)]">
                {plan.output_path}
              </code>
            </PlanRow>
            <PlanRow label={t("jinglePlanStems")}>
              <ul className="mt-1 space-y-0.5">
                {plan.stems.map((s, i) => (
                  <li
                    key={i}
                    className="font-mono text-ui-xs text-[var(--color-fg-muted)]"
                  >
                    {s.path}
                    {s.gain_db !== 0 &&
                      ` (${s.gain_db > 0 ? "+" : ""}${s.gain_db} dB)`}
                  </li>
                ))}
              </ul>
            </PlanRow>
            <PlanRow label={t("jinglePlanFfmpegArgs")}>
              <code className="block whitespace-pre-wrap break-all font-mono text-ui-xs text-[var(--color-fg-muted)]">
                {plan.ffmpeg_args.join(" ")}
              </code>
            </PlanRow>
          </dl>
        </section>
      )}
    </div>
  );
}

// ── Sub-components ────────────────────────────────────────────────────────────

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <label className="block">
      <div className="mb-1.5 flex items-baseline justify-between">
        <span className="text-ui-sm font-medium">{label}</span>
        {hint && (
          <span className="text-ui-xs text-[var(--color-fg-muted)]">
            {hint}
          </span>
        )}
      </div>
      {children}
    </label>
  );
}

function PlanRow({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="grid grid-cols-[6rem_1fr] gap-2">
      <dt className="text-ui-xs font-medium uppercase tracking-wider text-[var(--color-fg-muted)]">
        {label}
      </dt>
      <dd>{children}</dd>
    </div>
  );
}

function inputClass(invalid: boolean) {
  return [
    "w-full rounded-[var(--radius-md)] border bg-[var(--color-bg-surface)] px-3 py-2 text-ui-sm outline-none",
    "focus-visible:ring-1 focus-visible:ring-[var(--color-accent)]",
    invalid ? "border-[var(--color-danger)]" : "border-[var(--color-border)]",
  ].join(" ");
}

function selectClass(invalid: boolean) {
  return `${inputClass(invalid)} appearance-none pr-8`;
}

function capitalize(s: string) {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
