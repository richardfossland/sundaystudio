import { useEffect, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Loader2 } from "lucide-react";

import { Button } from "@/components/ui/Button";
import { ipc } from "@/lib/ipc";
import type { AudioSettings, LatencyZone } from "@/lib/bindings";

const SAMPLE_RATES = [44_100, 48_000, 88_200, 96_000];
const BUFFER_SIZES = [64, 128, 256, 512];

const ZONE_COLOR: Record<LatencyZone, string> = {
  green: "var(--color-success)",
  yellow: "var(--color-warning)",
  red: "var(--color-danger)",
};

/**
 * Settings → Audio (Phase 1.1). Pick input/output device, sample rate and
 * buffer size; see the round-trip latency those choices imply, colour-coded.
 *
 * Settings are loaded from and saved to the backend (an app-config JSON file);
 * in Phase 2.1 the active selection moves into the open project. The live
 * device-test meter and hot-plug toasts arrive in Phase 1.2/1.3 — they need an
 * open input stream (real hardware) to be meaningful.
 */
export function SettingsPage({ onBack }: { onBack?: () => void }) {
  const qc = useQueryClient();
  const devices = useQuery({
    queryKey: ["audio_devices"],
    queryFn: ipc.audio.devices,
  });
  const settings = useQuery({
    queryKey: ["audio_settings"],
    queryFn: ipc.audio.getSettings,
  });

  const [form, setForm] = useState<AudioSettings | null>(null);
  useEffect(() => {
    if (settings.data && !form) setForm(settings.data);
  }, [settings.data, form]);

  const latency = useQuery({
    queryKey: ["audio_latency", form?.sample_rate, form?.buffer_size],
    queryFn: () =>
      ipc.audio.latencyEstimate(form!.sample_rate, form!.buffer_size),
    enabled: !!form,
  });

  const save = useMutation({
    mutationFn: (next: AudioSettings) => ipc.audio.setSettings(next),
    onSuccess: (_data, next) => {
      qc.setQueryData(["audio_settings"], next);
    },
  });

  if (!form) {
    return (
      <Centered>
        {settings.isError ? (
          <p className="text-ui-sm text-[var(--color-fg-muted)]">
            Audio settings unavailable (running outside Tauri?).
          </p>
        ) : (
          <Loader2 className="animate-spin text-[var(--color-fg-muted)]" />
        )}
      </Centered>
    );
  }

  const update = (patch: Partial<AudioSettings>) =>
    setForm((f) => (f ? { ...f, ...patch } : f));

  const dirty = JSON.stringify(form) !== JSON.stringify(settings.data);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-2xl px-6 py-10">
        <header className="mb-8 flex items-center justify-between">
          <div>
            <div className="mb-1 text-ui-xs font-medium uppercase tracking-widest text-[var(--color-accent)]">
              Settings · Audio
            </div>
            <h1 className="text-ui-2xl font-bold">Audio device</h1>
          </div>
          {onBack && (
            <Button variant="ghost" size="sm" onClick={onBack}>
              ← Home
            </Button>
          )}
        </header>

        <div className="space-y-5 rounded-[var(--radius-lg)] border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-5">
          <Field label="Input device" hint="Microphone or USB interface">
            <DeviceSelect
              value={form.input_device}
              options={(devices.data?.inputs ?? []).map((d) => d.name)}
              onChange={(v) => update({ input_device: v })}
            />
          </Field>

          <Field label="Output device" hint="Headphones or speakers">
            <DeviceSelect
              value={form.output_device}
              options={(devices.data?.outputs ?? []).map((d) => d.name)}
              onChange={(v) => update({ output_device: v })}
            />
          </Field>

          <div className="grid grid-cols-2 gap-4">
            <Field label="Sample rate">
              <Native
                value={form.sample_rate}
                onChange={(v) => update({ sample_rate: Number(v) })}
                options={SAMPLE_RATES.map((r) => ({
                  value: r,
                  label: `${r / 1000} kHz`,
                }))}
              />
            </Field>
            <Field label="Buffer size" hint="Lower = less latency">
              <Native
                value={form.buffer_size}
                onChange={(v) => update({ buffer_size: Number(v) })}
                options={BUFFER_SIZES.map((b) => ({
                  value: b,
                  label: `${b} frames`,
                }))}
              />
            </Field>
          </div>

          {/* Latency readout */}
          <div className="flex items-center justify-between rounded-[var(--radius-md)] bg-[var(--color-bg-surface)] px-4 py-3">
            <div>
              <div className="text-ui-sm font-medium">
                Estimated round-trip latency
              </div>
              <div className="text-ui-xs text-[var(--color-fg-muted)]">
                Input + output buffers + driver. Live monitor figure refines in
                Phase 1.3.
              </div>
            </div>
            {latency.data && (
              <div className="flex items-center gap-2">
                <span
                  className="inline-block size-2.5 rounded-full"
                  style={{ background: ZONE_COLOR[latency.data.zone] }}
                />
                <span
                  className="font-mono text-ui-lg tabular-nums"
                  style={{ color: ZONE_COLOR[latency.data.zone] }}
                >
                  {latency.data.ms.toFixed(1)} ms
                </span>
              </div>
            )}
          </div>
        </div>

        <div className="mt-5 flex items-center gap-3">
          <Button
            variant="accent"
            onClick={() => save.mutate(form)}
            disabled={!dirty || save.isPending}
          >
            {save.isPending ? (
              <Loader2 size={16} className="animate-spin" />
            ) : save.isSuccess && !dirty ? (
              <Check size={16} />
            ) : null}
            {save.isPending ? "Saving…" : !dirty ? "Saved" : "Save changes"}
          </Button>
          {save.isError && (
            <span className="text-ui-sm text-[var(--color-danger)]">
              {save.error instanceof Error ? save.error.message : "Save failed"}
            </span>
          )}
        </div>
      </div>
    </div>
  );
}

function DeviceSelect({
  value,
  options,
  onChange,
}: {
  value: string | null;
  options: string[];
  onChange: (value: string | null) => void;
}) {
  return (
    <select
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value === "" ? null : e.target.value)}
      className="w-full rounded-[var(--radius-md)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-2 text-ui-sm outline-none focus-visible:ring-1 focus-visible:ring-[var(--color-accent)]"
    >
      <option value="">System default</option>
      {options.map((o) => (
        <option key={o} value={o}>
          {o}
        </option>
      ))}
      {/* Keep a persisted selection visible even if the device is unplugged. */}
      {value && !options.includes(value) && (
        <option value={value}>{value} (not connected)</option>
      )}
    </select>
  );
}

function Native({
  value,
  options,
  onChange,
}: {
  value: number;
  options: { value: number; label: string }[];
  onChange: (value: string) => void;
}) {
  return (
    <select
      value={value}
      onChange={(e) => onChange(e.target.value)}
      className="w-full rounded-[var(--radius-md)] border border-[var(--color-border)] bg-[var(--color-bg-surface)] px-3 py-2 text-ui-sm outline-none focus-visible:ring-1 focus-visible:ring-[var(--color-accent)]"
    >
      {options.map((o) => (
        <option key={o.value} value={o.value}>
          {o.label}
        </option>
      ))}
    </select>
  );
}

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

function Centered({ children }: { children: React.ReactNode }) {
  return <div className="grid h-full place-items-center">{children}</div>;
}
