//! AI feature wrappers — HTTP-based, deliberately isolated from the audio
//! engine (no AI call ever touches the real-time thread).
//!
//! Phase 5/6 add: auto-leveling, noise/breath/click cleanup, speaker isolation,
//! auto-ducking suggestions (Anthropic), and jingle music generation (Suno, via
//! an Edge Function so the key never reaches the client). Each AI feature is
//! opt-in with explicit first-use consent, and gated to the Pro tier.
//!
//! Empty in Phase 0.1.
