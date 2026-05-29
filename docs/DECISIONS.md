# Decisions — SundayStudio

A running log of architecture/product decisions and why we made them. Newest
first. Each entry: the decision, the context, and the alternative we rejected.

## ADR-0001 — Tauri 2 + Rust audio core (Phase 0.1)

**Decision.** Build on Tauri 2 with a Rust backend, matching SundayStage,
SundayPaper and Verbatim. The audio engine is native Rust (cpal + hound to
start, growing into a dedicated real-time thread).

**Context.** SundayStudio's hard problem is low-latency, reliable, multi-track
audio across macOS and Windows. That demands native code with real-time thread
control and direct access to CoreAudio/WASAPI — a web-audio or Electron-only
approach can't guarantee the deadlines. Rust gives us that control with
memory safety, and Tauri keeps the rest of the suite's stack (React 19,
Tailwind v4, TanStack Query, ts-rs bindings, the `AppError` IPC contract).

**Rejected.** Electron (heavier, no real-time thread story, and SundayRec
already occupies the Electron slot in the suite); a pure-web DAW (latency and
device-access ceilings); JUCE/C++ (no ecosystem reuse with the Sunday suite,
slower iteration).

## ADR-0002 — Prove the foundation before building the engine (Phase 0.1)

**Decision.** Phase 0.1 ships only two synchronous audio commands —
`audio_devices` (cpal enumeration) and `audio_record_test_tone` (hound WAV
write) — not any part of the real-time recorder.

**Context.** The plan is explicit that the engine is the project's biggest risk
and that Phase 1 should take weeks, not days. Before investing in the threading
model we want hard proof that cpal links and enumerates, and that we can write a
valid WAV, on both target platforms. These two commands are that proof, and
they double as the "Hello SundayStudio" bridge test.

**Rejected.** Scaffolding the full recorder in Phase 0 (premature — the
threading model deserves its own focused phase with real interfaces to test
against).

## ADR-0003 — Bundled effects only, no plugin hosting in v1

**Decision.** All effects are built in Rust and bundled. No VST3/AU/plugin
hosting in v1.

**Context.** Plugin hosting is precisely what makes GarageBand/Logic complex.
Our entire wedge is being the _simplest_ podcast tool. Bundling a small,
excellent set of voice/master effects keeps the surface small and the result
predictable. Plugin hosting is a v2 question, only if users demand it.

## ADR-0005 — App-level audio settings + a-priori latency estimate (Phase 1.1)

**Decision.** Audio settings (input/output device, sample rate, buffer size)
persist to a single JSON file in the app config dir, loaded/saved through pure,
unit-tested functions. The settings screen shows an _estimated_ round-trip
latency computed from buffer size and sample rate (`2 × buffer/rate + 2 ms`
driver allowance), colour-coded green/yellow/red.

**Context.** We need device selection and a latency figure before the
real-time engine exists. A pure estimate is honest for a settings screen shown
before any stream is open, and keeps Phase 1.1 fully testable with no hardware.
The _measured_ latency (queried from the live cpal stream) replaces the
estimate in Phase 1.3. Settings live at app level for now; Phase 2.1 moves the
active selection into the project (so reopening restores its interface) and
this file degrades to "last-used defaults".

**Rejected.** Storing in SQLite (no DB until Phase 2.1; a JSON file is simpler
and human-inspectable); failing hard on a corrupt settings file (audio config
is recoverable — we fall back to safe defaults instead).

## ADR-0009 — DSP effects tested by signal properties, not byte goldens (Phase 4.1)

**Decision.** The five bundled voice effects (gate, 4-band parametric EQ,
de-esser, compressor, saturator) are pure Rust implementing a common `Effect`
trait (prepare / process-in-place / reset), composed into a `VoiceChain` with
factory presets (Voice / Bright Voice / Warm Voice / Broadcast). They are tested
by _signal properties_ — frequency response, gain reduction above threshold,
soft-clip peak taming — with tolerances, not by exact output fingerprints.

**Context.** The audio golden harness (ADR-0002-era) fingerprints exact bytes,
which is right for the deterministic integer test tone but wrong for float DSP:
`sin`/`tanh`/`powf` differ in the last bits across platforms and compilers, so a
byte-exact golden would be flaky in CI. Property tests (a 1 kHz tone gains ~6 dB
through a +6 dB bell; 40 Hz is cut >6 dB by a voice preset's high-pass; a 4:1
compressor pulls a 0 dBFS tone down >8 dB) are both robust and meaningful, and
match the plan's "perceptually equivalent / within tolerance" bar.

The de-esser detects on a high-pass side-chain but ducks broadband: complementary
band recombination (`low = x − high`) leaks phase-shifted highs near the
crossover, weakening reduction; broadband ducking keyed on the sibilance band is
phase-coherent and is how most simple de-essers work.

**Rejected.** Byte-exact DSP goldens (cross-platform flaky); pulling in a DSP
framework (`fundsp`/`dasp`) — these effects are small, and hand-rolled RBJ
biquads + envelope followers keep the dependency surface and the math legible.

## ADR-0008 — Templates as backend data; recording page is a real shell (Phase 2.3)

**Decision.** The 8 quick-start templates are defined as pure data in Rust
(`project::templates`) and materialised by `apply`, so they're unit-tested and
there's one source of truth; the gallery renders them via `project_templates`.
New/Open use the native dialog (`tauri-plugin-dialog`). The recording page binds
real project tracks (arm/mute/solo/gain persist through the store) and chapters;
its transport record button is an explicit visual placeholder, because live
multi-track capture runs through the recorder's cpal stream and needs real
hardware (so meters read silence here, with an on-screen note).

**Context.** Templates are the on-ramp — most users should never see a blank
project. Keeping them backend-side means `apply` (project + pre-wired tracks) is
tested with no UI, and the gallery can't drift from what actually gets created.
The recording page is honest about the hardware boundary set in Phase 1.2:
everything that can work without a device (project/track/marker persistence)
does and is wired; the one thing that can't (capture) is clearly marked.

**Rejected.** Duplicating template metadata on the frontend (drift risk);
faking meters/levels on the recording page (would imply capture works when it
doesn't); a bespoke folder-picker (the native dialog is correct and accessible).

## ADR-0007 — `.scast` folder + SQLite tape model (Phase 2.1)

**Decision.** A project is a `.scast` folder: `manifest.json` + `project.sqlite`
(metadata only) + `takes/` (WAVs) + `edits/` + `exports/` + `cache/backups/`.
The database holds the tape model — Project → Track / Take → Region (+ Marker,
and forward-compat tables for effects/automation/jingles) — never audio. Access
is through sqlx runtime-checked queries; every mutation persists immediately
(no explicit "save"). Times/positions/durations are `f64` ms and rates/counts
`i32`, so the generated TypeScript stays plain `number` (no `bigint`).

**Context.** The "tape model" (takes are immutable; regions reference ranges
within them) is what makes editing non-destructive and keeps the database tiny
(< 5 MB) while `takes/` holds the gigabytes — directly answering GarageBand's
"files are too huge". Functions take `&SqlitePool`, so the whole store is
unit-tested against a throwaway temp database with no app or device. Recent
projects + hourly backup rotation (keep 5) live app-side, mirroring the audio
settings approach.

**Rejected.** A single opaque project file (can't stream gigabytes of audio in/
out, and bloats like GarageBand); compile-time-checked `query!` macros (would
require DATABASE_URL/`.sqlx` in CI — runtime queries keep the build hermetic);
`i64`/chrono timestamps (force `bigint` into the TS layer for no benefit here).

## ADR-0006 — Recorder split for testability without hardware (Phase 1.2)

**Decision.** The recording engine is split so the cpal device stream is the
only hardware-dependent piece. The audio callback's logic lives in
`CaptureSink::push_interleaved` (plain function, no device); `session` wires
per-channel rtrb rings to a writer thread that drains them to 24-bit WAVs. The
integration test drives `push_interleaved` with synthetic frames and asserts
the on-disk WAVs and meters — proving ring → writer → disk with no audio device.
`stream.rs` (opening the real cpal stream) is `pub`, wired, and documented as
hardware-unverified.

**Context.** This session has no audio hardware, and "recording reliability is
sacred" forbids shipping a blind, unverified real-time engine as done. Pushing
all the real-time-safe logic behind a device-free seam lets us unit-test the
risky data path (rings, draining, crash-safe flush, meters, sample counts) now,
and leaves a single, clearly-flagged surface (`stream.rs`) for hardware
validation in Phase 2.2 against the interface matrix in ARCHITECTURE.md.

**Rejected.** Wiring live `audio_record_start/stop` Tauri commands now (the
Stream lifecycle + `!Send` ownership + xrun behaviour genuinely need a device
in the loop — building it blind would be unverified and likely subtly wrong);
faking a device in tests (cpal's loopback/virtual-device story is platform-
specific and brittle — a device-free seam is simpler and proves more).

## ADR-0004 — AI is isolated from the audio engine, opt-in, Pro-gated

**Decision.** Every AI feature is HTTP-based (Anthropic for analysis/suggestions,
Suno for jingle music), runs off the audio thread, is opt-in with explicit
first-use consent, and lives behind the Pro tier. Provider keys live in Edge
Functions, never the client.

**Context.** Real-time audio and network calls have incompatible failure modes;
mixing them risks the recording. Building against the _abstraction_ (a music/
analysis provider) rather than a vendor lets us swap Suno for a competitor if
price or API changes — per the plan's afterword.
