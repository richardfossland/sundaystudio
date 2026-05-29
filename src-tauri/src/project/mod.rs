//! Project file format + persistence.
//!
//! Phase 2.1 implements the `.scast` project folder (manifest.json +
//! project.sqlite + takes/ + edits/ + exports/ + cache/) and the tape-model
//! domain (Project · Track · Take · Region · EffectInstance · Marker · ...).
//! Storage is local-first SQLite via sqlx (added behind the `db` feature).
//!
//! Empty in Phase 0.1.
