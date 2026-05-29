//! Built-in DSP effects. Bundled, real-time-safe, no third-party plugin hosting
//! (that is what keeps us simpler than GarageBand — see CLAUDE.md).
//!
//! Phase 4 fills this in, one real-time-safe module per effect:
//!   gate · eq (4-band parametric) · de_esser · compressor · saturator
//!   master chain · LUFS/EBU R128 normalisation
//!
//! Empty in Phase 0.1 — the module exists so the folder structure from the plan
//! is real from commit one.
