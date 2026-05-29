import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  AlertTriangle,
  ArrowLeft,
  CheckCircle2,
  CircleDot,
  Gauge,
  Loader2,
  Mic,
  Radio,
  Sliders,
  Volume2,
} from "lucide-react";

import { Brand } from "@/components/Brand";
import { Button } from "@/components/ui/Button";
import { ipc } from "@/lib/ipc";
import type {
  AudioDevice,
  LoudnessMeasurement,
  ToneResult,
} from "@/lib/bindings";

/** Format a LUFS/LU/dB value (or `—` when undefined / silent). */
function fmtDb(value: number | null, unit: string): string {
  return value === null ? "—" : `${value.toFixed(1)} ${unit}`;
}

/**
 * Diagnostics screen — the original "Hello SundayStudio" smoke checks, kept as
 * a dev/support tool reachable from the Start screen. Confirms the Rust ↔ React
 * bridge, lists audio devices (cpal), and writes a test-tone WAV to disk (hound).
 */
export function HomePage({ onBack }: { onBack?: () => void }) {
  const info = useQuery({ queryKey: ["app_info"], queryFn: ipc.app.info });
  const devices = useQuery({
    queryKey: ["audio_devices"],
    queryFn: ipc.audio.devices,
  });
  const presets = useQuery({
    queryKey: ["dsp_presets"],
    queryFn: ipc.dsp.presets,
  });
  const targets = useQuery({
    queryKey: ["dsp_loudness_targets"],
    queryFn: ipc.dsp.loudnessTargets,
  });
  const masterPresets = useQuery({
    queryKey: ["dsp_master_presets"],
    queryFn: ipc.dsp.masterPresets,
  });

  const [tone, setTone] = useState<ToneResult | null>(null);
  const [toneError, setToneError] = useState<string | null>(null);
  const [recording, setRecording] = useState(false);

  const [loudness, setLoudness] = useState<LoudnessMeasurement | null>(null);
  const [analyzing, setAnalyzing] = useState(false);
  const [analyzeError, setAnalyzeError] = useState<string | null>(null);

  async function recordTestTone() {
    setRecording(true);
    setToneError(null);
    setLoudness(null);
    setAnalyzeError(null);
    try {
      setTone(await ipc.audio.recordTestTone());
    } catch (err) {
      setToneError(err instanceof Error ? err.message : String(err));
    } finally {
      setRecording(false);
    }
  }

  async function analyzeTone() {
    if (!tone) return;
    setAnalyzing(true);
    setAnalyzeError(null);
    try {
      setLoudness(await ipc.dsp.analyzeFile(tone.path));
    } catch (err) {
      setAnalyzeError(err instanceof Error ? err.message : String(err));
    } finally {
      setAnalyzing(false);
    }
  }

  const backendOk = info.isSuccess;

  return (
    <div className="mx-auto flex min-h-screen max-w-2xl flex-col gap-8 px-6 py-12">
      <header className="flex items-center justify-between">
        <Brand size={32} />
        <div className="flex items-center gap-2">
          <span className="rounded-full border border-[var(--color-border)] px-2.5 py-1 text-ui-xs font-medium uppercase tracking-widest text-[var(--color-fg-muted)]">
            Diagnostics
          </span>
          {onBack && (
            <Button variant="ghost" size="sm" onClick={onBack}>
              <ArrowLeft size={15} />
              Back
            </Button>
          )}
        </div>
      </header>

      {/* Backend identity */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <StatusRow ok={backendOk} pending={info.isPending} label="Rust backend">
          {info.isSuccess && (
            <span className="font-mono text-ui-xs text-[var(--color-fg-muted)]">
              v{info.data.version} · Tauri {info.data.tauri_version} ·{" "}
              {info.data.platform}/{info.data.arch}
            </span>
          )}
          {info.isError && (
            <span className="text-ui-xs text-[var(--color-danger)]">
              IPC unavailable (running outside Tauri?)
            </span>
          )}
        </StatusRow>
        {info.isSuccess && (
          <p className="mt-2 text-ui-sm text-[var(--color-fg-muted)]">
            {info.data.greeting}
          </p>
        )}
      </section>

      {/* Audio devices */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <div className="mb-4 flex items-center justify-between">
          <h2 className="text-ui-md font-semibold">Audio devices</h2>
          {devices.isSuccess && (
            <span className="font-mono text-ui-xs text-[var(--color-fg-muted)]">
              host: {devices.data.host}
            </span>
          )}
        </div>

        {devices.isPending && <Muted>Scanning audio hardware…</Muted>}
        {devices.isError && (
          <Muted>
            Could not enumerate devices (running outside Tauri or no audio
            backend).
          </Muted>
        )}

        {devices.isSuccess && (
          <div className="grid gap-5 sm:grid-cols-2">
            <DeviceColumn
              icon={<Mic size={15} />}
              title="Inputs"
              devices={devices.data.inputs}
            />
            <DeviceColumn
              icon={<Volume2 size={15} />}
              title="Outputs"
              devices={devices.data.outputs}
            />
          </div>
        )}
      </section>

      {/* Test tone */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <h2 className="mb-1 text-ui-md font-semibold">Record test tone</h2>
        <p className="mb-4 text-ui-sm text-[var(--color-fg-muted)]">
          Writes a 1-second 440&nbsp;Hz sine wave to a WAV on disk — proof the
          recording-to-file path works.
        </p>

        <Button
          variant="accent"
          size="lg"
          onClick={recordTestTone}
          disabled={recording}
        >
          {recording ? (
            <Loader2 size={18} className="animate-spin" />
          ) : (
            <CircleDot size={18} />
          )}
          {recording ? "Writing…" : "Record test tone"}
        </Button>

        {tone && (
          <div className="mt-4 flex items-start gap-2 rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3">
            <CheckCircle2
              size={16}
              className="mt-0.5 shrink-0 text-[var(--color-success)]"
            />
            <div className="text-ui-xs">
              <div className="mb-1 font-medium text-[var(--color-fg)]">
                Wrote {(Number(tone.bytes) / 1024).toFixed(1)} KB ·{" "}
                {tone.sample_rate / 1000} kHz · {tone.duration_ms} ms
              </div>
              <code className="break-all font-mono text-[var(--color-fg-muted)]">
                {tone.path}
              </code>
            </div>
          </div>
        )}
        {toneError && (
          <div className="mt-4 flex items-start gap-2 rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3 text-ui-xs text-[var(--color-danger)]">
            <AlertTriangle size={16} className="mt-0.5 shrink-0" />
            <span className="break-all">{toneError}</span>
          </div>
        )}

        {/* Loudness analysis of the written tone (EBU R128, Phase 4.2) */}
        {tone && (
          <div className="mt-4 border-t border-[var(--color-border)] pt-4">
            <Button
              variant="surface"
              size="sm"
              onClick={analyzeTone}
              disabled={analyzing}
            >
              {analyzing ? (
                <Loader2 size={15} className="animate-spin" />
              ) : (
                <Gauge size={15} />
              )}
              {analyzing ? "Measuring…" : "Analyze loudness"}
            </Button>

            {loudness && (
              <dl className="mt-3 grid grid-cols-2 gap-x-4 gap-y-1.5 sm:grid-cols-3">
                <Metric
                  label="Integrated"
                  value={fmtDb(loudness.integrated_lufs, "LUFS")}
                />
                <Metric
                  label="Short-term"
                  value={fmtDb(loudness.short_term_lufs, "LUFS")}
                />
                <Metric
                  label="Momentary"
                  value={fmtDb(loudness.momentary_lufs, "LUFS")}
                />
                <Metric
                  label="Range"
                  value={fmtDb(loudness.loudness_range_lu, "LU")}
                />
                <Metric
                  label="True peak"
                  value={fmtDb(loudness.true_peak_dbtp, "dBTP")}
                />
                <Metric
                  label="Sample peak"
                  value={fmtDb(loudness.sample_peak_dbfs, "dBFS")}
                />
              </dl>
            )}
            {analyzeError && (
              <div className="mt-3 flex items-start gap-2 text-ui-xs text-[var(--color-danger)]">
                <AlertTriangle size={16} className="mt-0.5 shrink-0" />
                <span className="break-all">{analyzeError}</span>
              </div>
            )}
          </div>
        )}
      </section>

      {/* Loudness targets — "Match to platform" (Phase 4.2) */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <div className="mb-1 flex items-center gap-2">
          <Gauge size={15} className="text-[var(--color-fg-muted)]" />
          <h2 className="text-ui-md font-semibold">Loudness targets</h2>
        </div>
        <p className="mb-4 text-ui-sm text-[var(--color-fg-muted)]">
          Where each platform normalises your show. Master to the target and it
          plays back at the level you intended — no louder, no quieter.
        </p>
        {targets.isSuccess ? (
          <ul className="grid gap-2 sm:grid-cols-2">
            {targets.data.map((t) => (
              <li
                key={t.id}
                className="rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3"
              >
                <div className="flex items-baseline justify-between gap-2">
                  <span className="text-ui-sm font-medium">{t.label}</span>
                  <span className="font-mono text-ui-xs text-[var(--color-accent)]">
                    {t.integrated_lufs} LUFS
                  </span>
                </div>
                <div className="mt-0.5 text-ui-xs text-[var(--color-fg-muted)]">
                  {t.description}
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <Muted>Targets load when running in the SundayStudio app.</Muted>
        )}
      </section>

      {/* Mastering presets (Phase 4.2) */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <div className="mb-1 flex items-center gap-2">
          <Radio size={15} className="text-[var(--color-fg-muted)]" />
          <h2 className="text-ui-md font-semibold">Mastering presets</h2>
        </div>
        <p className="mb-4 text-ui-sm text-[var(--color-fg-muted)]">
          One-click master chains (EQ · 3-band compressor · brick-wall limiter)
          paired with a loudness target. They run on the final mix at export.
        </p>
        {masterPresets.isSuccess ? (
          <ul className="grid gap-2 sm:grid-cols-2">
            {masterPresets.data.map((p) => (
              <li
                key={p.id}
                className="rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3"
              >
                <div className="text-ui-sm font-medium">{p.label}</div>
                <div className="mt-0.5 text-ui-xs text-[var(--color-fg-muted)]">
                  {p.description}
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <Muted>Presets load when running in the SundayStudio app.</Muted>
        )}
      </section>

      {/* Bundled voice presets (DSP engine, Phase 4.1) */}
      <section className="rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
        <div className="mb-1 flex items-center gap-2">
          <Sliders size={15} className="text-[var(--color-fg-muted)]" />
          <h2 className="text-ui-md font-semibold">Voice presets</h2>
        </div>
        <p className="mb-4 text-ui-sm text-[var(--color-fg-muted)]">
          Bundled processing chains (gate · EQ · de-esser · compressor ·
          saturator). They attach to mixer tracks in a later phase.
        </p>
        {presets.isSuccess ? (
          <ul className="grid gap-2 sm:grid-cols-2">
            {presets.data.map((p) => (
              <li
                key={p.id}
                className="rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] p-3"
              >
                <div className="text-ui-sm font-medium">{p.label}</div>
                <div className="mt-0.5 text-ui-xs text-[var(--color-fg-muted)]">
                  {p.description}
                </div>
              </li>
            ))}
          </ul>
        ) : (
          <Muted>Presets load when running in the SundayStudio app.</Muted>
        )}
      </section>
    </div>
  );
}

function StatusRow({
  ok,
  pending,
  label,
  children,
}: {
  ok: boolean;
  pending: boolean;
  label: string;
  children?: React.ReactNode;
}) {
  return (
    <div className="flex items-center justify-between gap-3">
      <div className="flex items-center gap-2">
        <span
          className="inline-block size-2 rounded-full"
          style={{
            background: pending
              ? "var(--color-warning)"
              : ok
                ? "var(--color-success)"
                : "var(--color-danger)",
          }}
        />
        <span className="text-ui-sm font-medium">{label}</span>
      </div>
      {children}
    </div>
  );
}

function DeviceColumn({
  icon,
  title,
  devices,
}: {
  icon: React.ReactNode;
  title: string;
  devices: AudioDevice[];
}) {
  return (
    <div>
      <div className="mb-2 flex items-center gap-1.5 text-ui-xs font-medium uppercase tracking-wider text-[var(--color-fg-muted)]">
        {icon}
        {title} · {devices.length}
      </div>
      {devices.length === 0 ? (
        <Muted>None found</Muted>
      ) : (
        <ul className="flex flex-col gap-1.5">
          {devices.map((d) => (
            <li
              key={`${d.direction}:${d.name}`}
              className="rounded-[var(--radius-sm)] bg-[var(--color-bg-surface)] px-2.5 py-1.5"
            >
              <div className="flex items-center gap-1.5 text-ui-sm">
                <span className="truncate">{d.name}</span>
                {d.is_default && (
                  <span className="shrink-0 rounded-sm bg-[var(--color-accent)] px-1 text-[10px] font-semibold uppercase text-[var(--color-accent-fg)]">
                    default
                  </span>
                )}
              </div>
              <div className="mt-0.5 font-mono text-[11px] text-[var(--color-fg-muted)]">
                {d.channels} ch
                {d.sample_rates.length > 0 &&
                  ` · ${d.sample_rates.map((r) => r / 1000).join("/")} kHz`}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <dt className="text-[10px] font-medium uppercase tracking-wider text-[var(--color-fg-muted)]">
        {label}
      </dt>
      <dd className="font-mono text-ui-sm">{value}</dd>
    </div>
  );
}

function Muted({ children }: { children: React.ReactNode }) {
  return <p className="text-ui-sm text-[var(--color-fg-muted)]">{children}</p>;
}
