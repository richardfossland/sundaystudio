# CLAUDE.md — SundayStudio

SundayStudio is the podcast & jingle production app of the Sunday suite — the
simplest professional podcast producer on the market. Simpler than GarageBand,
more capable than Audacity, far cheaper than Audition or Hindenburg.

Built for the church that wants a conversation podcast running _this week_:
5–8 microphones at once, AI-driven cleanup and leveling, a generated jingle in
under a minute, a finished LUFS-normalized MP3 ready for the podcast platforms.

## Product north star

```
SundayStudio is a podcast and jingle production application for churches.

Target user: a church volunteer or staff member who wants to record a
conversation podcast (sermon recap, ministry discussion, interview series)
and ship it without learning a real DAW. They have 1-8 microphones, a USB
interface, and 30 minutes.

Core promises:
1. Simpler than GarageBand. Truly. No instruments, no synthesis, no MIDI,
   no music creation surface. Just audio.
2. Powerful enough for serious podcast work: 8 simultaneous tracks, proper
   monitoring, broadcast-quality bundled effects.
3. AI does the boring audio engineering: leveling, noise removal, ducking,
   loudness normalization.
4. Jingle creation in under 60 seconds using AI music generation + smart
   voiceover templates.
5. Direct to podcast platform: LUFS-normalized export, RSS-ready, integrated
   with Spotify for Podcasters and Apple Podcasts.

Competitive positioning:
- vs GarageBand: podcast-first, not music-first; smaller project files;
  auto-mute and auto-duck built in
- vs Audacity: modern UI, AI features, much friendlier
- vs Hindenburg (~$95-300): one-time-cheap or subscription, AI-native
- vs Adobe Audition / Creative Cloud: a fraction of the price, focused,
  no learning curve
- vs Riverside / Squadcast: not remote-recording focused; we're for
  in-person church recording (sermons, panel discussions, Sunday school
  conversations)

Tech principles:
- Low-latency audio is non-negotiable. < 10ms monitor round-trip on common
  hardware.
- Recording reliability is sacred. A 90-minute recording must NEVER be
  lost to a crash. Continuous write to disk, watchdog process, recovery
  on restart.
- AI is opt-in per feature, with explicit consent the first time.
- Effects are bundled. NO third-party plugin support in v1 — that is what
  keeps us simpler than GarageBand. Plugin hosting is v2 if users demand.
- Sunday Pro tier unlocks AI, jingle generation, and direct platform upload.

Out of scope for v1 (most are revisit-after-launch):
- VST3/AU plugin hosting
- MIDI and software instruments
- Video recording (podcast video is a different problem; see SundayRec)
- Remote recording (Riverside-style)
- Loop-based music creation
```

## The non-negotiable: the audio engine

The single biggest technical risk is real-time multi-track audio I/O across
macOS and Windows. The whole product collapses if the foundation is not
rock-solid. See `docs/ARCHITECTURE.md` for the engine design. The rules:

- The **audio thread** (cpal callback, real-time priority) NEVER allocates,
  NEVER locks, NEVER blocks, NEVER touches disk. It only moves samples through
  lock-free ring buffers and writes meter peaks to atomics.
- A separate **writer thread** owns all disk I/O, draining the ring buffers to
  per-track WAVs with incremental headers so a crash leaves a playable file.
- The **UI** never talks to the audio thread directly: commands go through a
  lock-free queue; meters are read from atomics.

## Stack

- **Tauri 2** (Rust backend) + React 19 + TypeScript + Tailwind CSS v4
- **shadcn/ui**-style primitives, customized to our tokens
- **TanStack Query** (server/IPC state) + **Zustand** (UI state)
- Rust audio crates (added per phase): `cpal` (I/O), `hound` (WAV), `rtrb`
  (lock-free ring buffer), `rubato` (resampling), `symphonia` (decode),
  `dasp`/`fundsp` (DSP), `ebur128` (loudness)
- **SQLite** via `sqlx` for project storage (Phase 2.1)
- **ffmpeg** bundled sidecar for final-stage MP3/AAC encoding (Phase 7)
- AI: Anthropic API (leveling/cleanup suggestions, show notes) + Suno API
  (jingle music) — HTTP, behind Edge Functions, isolated from the audio engine
- **ts-rs** generates the TypeScript bindings from Rust types
  (`cargo test export_bindings` from `src-tauri/`)
- Languages: Norwegian + English at launch; Swedish, Danish, German, French,
  Polish within 60 days (match the Sunday family)

## Layout

```
src/
  app/         page-level shells
  features/    record · edit · jingle · settings · diagnostics
  components/  shared UI (Brand, ui/*)
  lib/         ipc (typed invoke wrappers), bindings (ts-rs), hooks, cn, theme
  styles/      tokens.css (Sunday Gold accent + audio palette), globals.css
src-tauri/
  src/
    audio/     the engine — devices (0.1), recorder/mixer/monitor (Phase 1)
    dsp/       built-in effects (Phase 4)
    project/   project file format + SQLite (Phase 2.1)
    export/    encoding + platform export (Phase 7)
    ai/        Anthropic / Suno wrappers (Phase 5/6)
    commands/  thin Tauri IPC handlers (entity_verb)
    services/  cross-cutting logic
```

## Conventions

- Commands are named `entity_verb` (`app_info`, `audio_devices`,
  `audio_record_test_tone`). They are thin and delegate to the domain modules.
- Every command returns `Result<T, AppError>`; `AppError` serializes to
  `{ code, message }`. Keep `AppError::code()` (Rust) and the `AppError` union
  in `src/lib/bindings/index.ts` in sync.
- TypeScript: strict, no `any`, no unused vars (allow `_`-prefixed).
- Conventional Commits. `npm run check` runs the full gate
  (lint + typecheck + vitest + clippy + cargo test).
- Dark-first; light mode equally polished. Accent is **Sunday Gold** (#D4A73A).

## Tier model

- **Free:** 2 tracks, basic effects, manual everything, no jingle AI,
  watermark on exports.
- **Sunday Cast Pro:** 8 tracks, all effects + AI features, jingle AI,
  no watermark.
- **Sunday Cast Studio:** + team account, more jingle generations, direct
  platform upload.

## Three things to guard (from the plan's afterword)

1. **Do not drift into "real DAW" territory.** Say no to MIDI, instruments,
   and plugin hosting for the first 12 months. We win by being the world's
   _simplest podcast tool_, not the second-simplest DAW.
2. **The jingle feature sells the product.** Lead marketing with jingle-AI,
   not recording. Multi-track recording is table stakes; jingle-AI is the wow.
3. **Trust Suno (or a competitor) for music generation — don't build it.**
   Build a great wrapper around the abstraction, not the vendor, so providers
   can be swapped.
