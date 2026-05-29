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

## ADR-0004 — AI is isolated from the audio engine, opt-in, Pro-gated

**Decision.** Every AI feature is HTTP-based (Anthropic for analysis/suggestions,
Suno for jingle music), runs off the audio thread, is opt-in with explicit
first-use consent, and lives behind the Pro tier. Provider keys live in Edge
Functions, never the client.

**Context.** Real-time audio and network calls have incompatible failure modes;
mixing them risks the recording. Building against the _abstraction_ (a music/
analysis provider) rather than a vendor lets us swap Suno for a competitor if
price or API changes — per the plan's afterword.
