/**
 * /design — a living style guide. Renders every design token and audio
 * primitive with realistic, interactive state so the look can be eyeballed and
 * regressions caught in one place.
 */
import { useEffect, useRef, useState, type ReactNode } from "react";

import { Brand } from "@/components/Brand";
import { ThemeToggle } from "@/components/ThemeToggle";
import { Button } from "@/components/ui/Button";
import {
  JingleCard,
  LevelMeter,
  LevelReadout,
  RecordButton,
  Timecode,
  TrackHeader,
  Waveform,
  fakePeaks,
  type RecordState,
  type TrackState,
} from "@/components/audio";

const SEMANTIC = [
  "--color-bg",
  "--color-bg-elevated",
  "--color-bg-surface",
  "--color-fg",
  "--color-fg-muted",
  "--color-border",
  "--color-accent",
  "--color-brand",
];
const AUDIO = [
  "--color-waveform",
  "--color-waveform-peak",
  "--color-meter-green",
  "--color-meter-yellow",
  "--color-meter-red",
  "--color-recording",
];
const STATUS = [
  "--color-success",
  "--color-warning",
  "--color-danger",
  "--color-info",
];
const UI_SCALE = ["xs", "sm", "md", "lg", "xl", "2xl", "3xl"] as const;

const TRACK_COLORS = [
  "var(--color-gold-400)",
  "var(--color-info)",
  "var(--color-success)",
  "var(--color-meter-yellow)",
];

export function DesignPage({ onBack }: { onBack?: () => void }) {
  // A simulated live meter so the meters/animations actually move.
  const level = useSimulatedLevel();

  const [recordState, setRecordState] = useState<RecordState>("idle");
  const [position, setPosition] = useState(3_661_007);
  const [tracks, setTracks] = useState<TrackState[]>(() =>
    ["Host — Pastor Lars", "Guest — Maria", "Music bed"].map((name, i) => ({
      name,
      color: TRACK_COLORS[i],
      armed: i < 2,
      muted: false,
      soloed: false,
      monitoring: i === 0,
      gainDb: i === 2 ? -8 : 0,
      levelDb: -60,
      peakDb: -60,
    })),
  );

  // Drive each track's meter from the simulated level, offset per track.
  useEffect(() => {
    setTracks((prev) =>
      prev.map((t, i) => {
        const db = t.muted ? -60 : level - i * 6;
        return { ...t, levelDb: db, peakDb: Math.max(t.peakDb ?? -60, db) };
      }),
    );
  }, [level]);

  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-4xl px-8 py-10">
        <header className="mb-10 flex items-start justify-between">
          <div>
            <div className="mb-1 text-ui-xs font-medium uppercase tracking-widest text-[var(--color-accent)]">
              Design system
            </div>
            <Brand size={34} />
            <p className="mt-2 text-ui-sm text-[var(--color-fg-muted)]">
              Living style guide — tokens and audio primitives.
            </p>
          </div>
          <div className="flex items-center gap-2">
            <ThemeToggle />
            {onBack && (
              <Button variant="ghost" size="sm" onClick={onBack}>
                ← Home
              </Button>
            )}
          </div>
        </header>

        <Section title="Colours — semantic">
          <SwatchGrid tokens={SEMANTIC} />
        </Section>
        <Section title="Colours — audio domain">
          <SwatchGrid tokens={AUDIO} />
        </Section>
        <Section title="Colours — status">
          <SwatchGrid tokens={STATUS} />
        </Section>

        <Section title="Typography">
          <div className="space-y-1">
            {UI_SCALE.map((s) => (
              <div
                key={s}
                style={{ fontSize: `var(--text-ui-${s})` }}
                className="font-sans"
              >
                UI {s} — The church podcast that ships this week.
              </div>
            ))}
            <div className="pt-2 font-mono text-ui-sm text-[var(--color-fg-muted)]">
              Mono — 00:42:50.120 · -14.0 LUFS · 48 kHz
            </div>
          </div>
        </Section>

        <Section title="Buttons">
          <div className="flex flex-wrap items-center gap-3">
            <Button variant="accent">Accent</Button>
            <Button variant="surface">Surface</Button>
            <Button variant="ghost">Ghost</Button>
            <Button variant="accent" size="sm">
              Small
            </Button>
            <Button variant="accent" size="lg">
              Large
            </Button>
            <Button variant="surface" disabled>
              Disabled
            </Button>
          </div>
        </Section>

        <Section title="RecordButton">
          <div className="flex items-center gap-8">
            <div className="flex flex-col items-center gap-2">
              <RecordButton
                state={recordState}
                onClick={() =>
                  setRecordState((s) =>
                    s === "recording"
                      ? "idle"
                      : s === "armed"
                        ? "recording"
                        : "armed",
                  )
                }
              />
              <span className="text-ui-xs text-[var(--color-fg-muted)]">
                click to cycle · {recordState}
              </span>
            </div>
            <div className="flex items-end gap-6">
              {(["idle", "armed", "recording"] as const).map((st) => (
                <div key={st} className="flex flex-col items-center gap-2">
                  <RecordButton state={st} size={48} />
                  <span className="text-ui-xs text-[var(--color-fg-muted)]">
                    {st}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </Section>

        <Section title="LevelMeter">
          <div className="flex items-end gap-6">
            <div className="flex h-32 items-end gap-2">
              <LevelMeter db={level} peakDb={-3} className="h-full" />
              <LevelMeter db={level - 8} peakDb={-10} className="h-full" />
              <LevelMeter db={level - 20} className="h-full" />
            </div>
            <div className="flex-1 space-y-3">
              <LevelMeter db={level} peakDb={-3} orientation="horizontal" />
              <div className="flex items-center gap-3">
                <LevelReadout db={level} />
                <LevelReadout db={-3} />
                <LevelReadout db={-60} />
              </div>
            </div>
          </div>
        </Section>

        <Section title="Timecode">
          <div className="flex items-center gap-8">
            <Timecode ms={position} size="lg" onCommit={setPosition} />
            <div className="text-ui-xs text-[var(--color-fg-muted)]">
              click to edit · accepts 1:30, 90, 01:01:01.007
            </div>
          </div>
        </Section>

        <Section title="TrackHeader">
          <div className="space-y-2">
            {tracks.map((t, i) => (
              <TrackHeader
                key={t.name}
                track={t}
                onChange={(patch) =>
                  setTracks((prev) =>
                    prev.map((x, j) => (j === i ? { ...x, ...patch } : x)),
                  )
                }
              />
            ))}
          </div>
        </Section>

        <Section title="Waveform">
          <div className="space-y-3">
            <Waveform peaks={fakePeaks(220, 7)} progress={0.42} />
            <Waveform peaks={fakePeaks(80, 99)} variant="mini" />
          </div>
        </Section>

        <Section title="JingleCard">
          <div className="grid gap-3 sm:grid-cols-3">
            <JingleCard
              name="Welcome intro"
              durationMs={15_000}
              peaks={fakePeaks(60, 3)}
              variants={2}
            />
            <JingleCard
              name="Episode outro"
              durationMs={10_000}
              peaks={fakePeaks(60, 21)}
            />
            <JingleCard
              name="Mid-episode bumper"
              durationMs={5_000}
              peaks={fakePeaks(60, 42)}
            />
          </div>
        </Section>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <section className="mb-10">
      <h2 className="mb-3 text-ui-xs font-semibold uppercase tracking-wider text-[var(--color-fg-muted)]">
        {title}
      </h2>
      {children}
    </section>
  );
}

function SwatchGrid({ tokens }: { tokens: string[] }) {
  return (
    <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
      {tokens.map((v) => (
        <div
          key={v}
          className="overflow-hidden rounded-[var(--radius-md)] border border-[var(--color-border)]"
        >
          <div className="h-12" style={{ background: `var(${v})` }} />
          <div className="bg-[var(--color-bg-elevated)] px-2 py-1 font-mono text-[10px] text-[var(--color-fg-muted)]">
            {v}
          </div>
        </div>
      ))}
    </div>
  );
}

/** A gently oscillating dBFS level (~ -28..-2) for demoing live meters. */
function useSimulatedLevel(): number {
  const [db, setDb] = useState(-24);
  const t = useRef(0);
  useEffect(() => {
    const id = window.setInterval(() => {
      t.current += 0.15;
      // Two sines + a little variation, mapped into a speech-like range.
      const v = Math.sin(t.current) * 0.5 + Math.sin(t.current * 2.3) * 0.3;
      setDb(-15 + v * 13);
    }, 60);
    return () => window.clearInterval(id);
  }, []);
  return db;
}
