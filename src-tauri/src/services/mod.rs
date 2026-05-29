//! Cross-cutting business logic that does not belong to a single domain module.
//! Commands stay thin and delegate here (or to `audio` / `project` / `export`).
//!
//! Planned: `db` (SQLite pool + migrations, Phase 2.1), `account` (Sunday OAuth,
//! Phase 8), `quota` (AI generation tracking, Phase 6.2).
//!
//! Empty in Phase 0.1.
