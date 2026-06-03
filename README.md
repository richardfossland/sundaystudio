# SundayStudio

Podcast & jingle production for churches — part of the [Sunday suite](../).

The simplest professional podcast producer: 5–8 microphones at once, AI-driven
cleanup and leveling, a generated jingle in under a minute, and a finished
LUFS-normalized MP3 ready for Spotify and Apple Podcasts. Simpler than
GarageBand, friendlier than Audacity, far cheaper than Hindenburg or Audition.

> **Status:** Phase 7.1b. The foundation, the multi-track recorder, the
> waveform editor, the bundled DSP + mastering/loudness chain, the AI leveling
> and jingle wrappers, and the export pipeline (mastered WAV bounce + ffmpeg
> encode step) are all in place. The remaining gaps are the parts that need
> real hardware, the ffmpeg binary, or live API keys to exercise — see
> [Known offline limitations](#known-offline-limitations).

## Completed phases

Each phase is recorded in [`docs/DECISIONS.md`](./docs/DECISIONS.md) (ADRs) and
the matching `feat(...)` commit in the log.

- [x] **0.1 — Foundation.** Tauri 2 + Rust audio core; `audio_devices`
      (cpal enumeration) + `audio_record_test_tone` (hound WAV write) prove the
      stack links; design tokens + `/design` route.
- [x] **1.1 — Audio settings.** Device selection, sample rate / buffer size,
      a-priori round-trip latency estimate (pure, unit-tested).
- [x] **1.2 — Recorder skeleton.** Multi-track recording engine split so the
      cpal stream is the only hardware-bound seam; session pipeline is testable.
- [x] **1.3 — Monitoring mixer.** Low-latency monitor with soft mute/solo.
- [x] **1.4 — Playback engine.** Timeline playback transport.
- [x] **2.1 — Project format.** `.scast` folder (`manifest.json` +
      `project.sqlite`) with the SQLite tape model; save/load registry.
- [x] **2.2 — Live recording transport** wired to the session engine.
- [x] **2.3 — Quick-start templates + recording UI shell** (8 templates as
      backend data).
- [x] **3.1 — Waveform timeline editor.**
- [x] **3.2 — Editing ops.** Command algebra + undo/redo; merge, crossfade,
      cut/copy/paste; region-aware export (bounce the timeline, not whole takes).
- [x] **3.3 — Silence detection + removal** (level-based).
- [x] **4.1 — Bundled voice DSP effects** (gate, 4-band EQ, …) tested by signal
      properties.
- [x] **4.2 — Loudness + master chain.** `ebur128` LUFS / true-peak measurement,
      clip-safe normalization, multiband compressor + brick-wall limiter,
      mastering presets paired with platform loudness targets (surfaced on
      Diagnostics).
- [x] **4.3 — Per-track voice processing** applied on export.
- [x] **5.1 — AI auto-leveling** via the Anthropic API wrapper.
- [x] **6 — Jingle generation.** Offline jingle spec pipeline + Suno wrapper;
      Norwegian/English i18n.
- [x] **7.1a — Export bounce.** Pure renderer mixes the take's per-track WAVs,
      runs the master chain, loudness-normalises, writes a 24-bit master WAV.
- [x] **7.1b — Encode step.** Deterministic ffmpeg arg builder + sidecar call
      for lossy/archival masters (MP3/AAC/FLAC); falls back to the master WAV
      when ffmpeg is unavailable.

## Known offline limitations

The audio engine is built and unit/integration-tested everywhere it can be
without a rig, but three seams are deliberately **unverified** in this build
because they need resources this environment doesn't have. Each is isolated so
the rest of the gate stays green:

- **Real audio I/O — `src-tauri/src/audio/recorder/stream.rs`.** The cpal input
  stream is the one part of the recorder that needs real hardware. It compiles
  and is wired to the tested session pipeline (`CaptureSink`), but is marked
  `HARDWARE-UNVERIFIED` and must still be validated on actual interfaces
  (8-track 60-min capture, device unplug, sample-rate mismatch, crash recovery).
- **ffmpeg encoding — `src-tauri/src/export/encode.rs`.** `EncodePlan` /
  `build_ffmpeg_args` are pure and fully tested; `encode_with_ffmpeg` is the
  only impure entry point and needs the bundled ffmpeg binary, which isn't
  present here. With ffmpeg absent it returns the native master WAV with a note,
  so export never blindly fails — the actual transcode is exercised on a rig.
- **AI wrappers (`src-tauri/src/ai/`).** Leveling (Anthropic) and jingle music
  (Suno) are HTTP-based and opt-in; their offline-testable logic (request
  building, parsing, spec pipeline) is covered, but the live calls require API
  keys and are not exercised in the gate.

## Stack

Tauri 2 · Rust (cpal + hound, growing into a real-time audio engine) · React 19
· TypeScript · Tailwind CSS v4 · TanStack Query · Zustand · ts-rs bindings.

See [`CLAUDE.md`](./CLAUDE.md) for the product north star and conventions, and
[`docs/ARCHITECTURE.md`](./docs/ARCHITECTURE.md) for the audio engine design.

## Develop

```bash
npm install            # install frontend deps (also sets up husky)
npm run tauri dev      # run the desktop app (Rust backend + Vite frontend)
```

## Quality gate

```bash
npm run check          # lint + typecheck + vitest + clippy + cargo test
```

Individually:

```bash
npm run lint           # eslint
npm run typecheck      # tsc --noEmit
npm run test           # vitest (unit/integration)
npm run test:e2e       # playwright (app-shell smoke)
npm run test:rust      # cargo test (audio core: devices + tone)
npm run lint:rust      # cargo clippy -D warnings
```

TypeScript bindings are generated from the Rust types:

```bash
cargo test --manifest-path src-tauri/Cargo.toml export_bindings
```

## License

Proprietary — © Richard Fossland. Part of the Sunday suite.
