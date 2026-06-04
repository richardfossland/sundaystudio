//! Project registry (Phase 2.1) — an app-level index of all known `.scast`
//! projects.
//!
//! The per-project SQLite files hold audio metadata; the *registry* answers
//! the question "what projects does this user have?" without opening every
//! file. It is stored as `<config_dir>/registry.json`.
//!
//! The registry exposes five operations the Tauri commands delegate to:
//!   `new`     — create a project in the user's data dir (no file dialog)
//!   `save`    — update an existing project's mutable metadata in the registry
//!   `load`    — open a single project by id and return its metadata + snapshot
//!   `list`    — return all registry entries as `ProjectMeta`
//!   `delete`  — remove an entry from the registry (and optionally the folder)
//!
//! The `scast` module still handles the folder layout and per-project SQLite;
//! the registry only indexes paths and caches name/date for fast listing.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use uuid::Uuid;

use super::fsutil::atomic_write;
use crate::error::{AppError, AppResult};

const REGISTRY_FILE: &str = "registry.json";

/// One entry in the app-level project registry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ProjectMeta.ts")]
pub struct ProjectMeta {
    /// Stable identifier (UUIDv4, not the per-project UUID stored in SQLite).
    pub id: String,
    /// Human-readable project name (mirrors `Project.name`).
    pub name: String,
    /// Absolute path to the `.scast` folder.
    pub path: String,
    /// Epoch ms the project was created.
    pub created_at: f64,
    /// Epoch ms the registry entry was last modified (rename / save).
    pub updated_at: f64,
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn registry_path(config_dir: &Path) -> PathBuf {
    config_dir.join(REGISTRY_FILE)
}

/// Load the registry list. Missing or corrupt file → empty list.
pub fn load_all(config_dir: &Path) -> Vec<ProjectMeta> {
    match fs::read(registry_path(config_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn save_all(config_dir: &Path, entries: &[ProjectMeta]) -> AppResult<()> {
    // Atomic write: a crash mid-write must leave the previous registry intact,
    // not a truncated file that `load_all` would read as an empty list — which
    // would silently drop every known project from the home screen.
    atomic_write(
        &registry_path(config_dir),
        &serde_json::to_vec_pretty(entries)?,
    )
}

/// Register a new project and persist the registry. Returns the new entry.
pub fn register(config_dir: &Path, name: &str, scast_path: &Path) -> AppResult<ProjectMeta> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("project name cannot be empty".into()));
    }
    let now = now_ms();
    let entry = ProjectMeta {
        id: Uuid::now_v7().to_string(),
        name: name.to_string(),
        path: scast_path.to_string_lossy().into_owned(),
        created_at: now,
        updated_at: now,
    };
    let mut list = load_all(config_dir);
    list.insert(0, entry.clone());
    save_all(config_dir, &list)?;
    Ok(entry)
}

/// Find an entry by registry id.
pub fn find(config_dir: &Path, id: &str) -> AppResult<ProjectMeta> {
    load_all(config_dir)
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::NotFound {
            entity: "project",
            id: id.to_string(),
        })
}

/// Update the mutable fields (name, updated_at) of an existing entry.
pub fn update_meta(config_dir: &Path, id: &str, new_name: &str) -> AppResult<ProjectMeta> {
    let new_name = new_name.trim();
    if new_name.is_empty() {
        return Err(AppError::Validation("project name cannot be empty".into()));
    }
    let mut list = load_all(config_dir);
    let entry = list
        .iter_mut()
        .find(|e| e.id == id)
        .ok_or_else(|| AppError::NotFound {
            entity: "project",
            id: id.to_string(),
        })?;
    entry.name = new_name.to_string();
    entry.updated_at = now_ms();
    let updated = entry.clone();
    save_all(config_dir, &list)?;
    Ok(updated)
}

/// Remove an entry from the registry. Does NOT touch the `.scast` folder.
pub fn remove(config_dir: &Path, id: &str) -> AppResult<()> {
    let mut list = load_all(config_dir);
    let before = list.len();
    list.retain(|e| e.id != id);
    if list.len() == before {
        return Err(AppError::NotFound {
            entity: "project",
            id: id.to_string(),
        });
    }
    save_all(config_dir, &list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    // ── list / register ───────────────────────────────────────────────────────

    #[test]
    fn empty_registry_returns_empty_list() {
        let dir = tmp();
        assert!(load_all(dir.path()).is_empty());
    }

    #[test]
    fn register_creates_entry_with_unique_id() {
        let dir = tmp();
        let a = register(dir.path(), "Alpha", Path::new("/a.scast")).unwrap();
        let b = register(dir.path(), "Beta", Path::new("/b.scast")).unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.name, "Alpha");
        assert_eq!(b.name, "Beta");
    }

    #[test]
    fn register_persists_across_loads() {
        let dir = tmp();
        let entry = register(dir.path(), "Sunday Recap", Path::new("/sr.scast")).unwrap();
        let list = load_all(dir.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, entry.id);
        assert_eq!(list[0].name, "Sunday Recap");
        assert_eq!(list[0].path, "/sr.scast");
    }

    #[test]
    fn register_newest_first() {
        let dir = tmp();
        register(dir.path(), "First", Path::new("/a.scast")).unwrap();
        register(dir.path(), "Second", Path::new("/b.scast")).unwrap();
        let list = load_all(dir.path());
        // Most recent is at index 0.
        assert_eq!(list[0].name, "Second");
        assert_eq!(list[1].name, "First");
    }

    #[test]
    fn register_rejects_empty_name() {
        let dir = tmp();
        assert!(register(dir.path(), "  ", Path::new("/a.scast")).is_err());
        assert!(register(dir.path(), "", Path::new("/a.scast")).is_err());
    }

    // ── find ──────────────────────────────────────────────────────────────────

    #[test]
    fn find_returns_entry_by_id() {
        let dir = tmp();
        let entry = register(dir.path(), "My Podcast", Path::new("/p.scast")).unwrap();
        let found = find(dir.path(), &entry.id).unwrap();
        assert_eq!(found, entry);
    }

    #[test]
    fn find_errors_on_missing_id() {
        let dir = tmp();
        assert!(find(dir.path(), "no-such-id").is_err());
    }

    // ── update_meta ───────────────────────────────────────────────────────────

    #[test]
    fn update_meta_changes_name_and_bumps_updated_at() {
        let dir = tmp();
        let entry = register(dir.path(), "Old Name", Path::new("/p.scast")).unwrap();
        let updated = update_meta(dir.path(), &entry.id, "New Name").unwrap();
        assert_eq!(updated.name, "New Name");
        assert_eq!(updated.id, entry.id);
        assert!(updated.updated_at >= entry.updated_at);
        // Verify the list was persisted.
        let list = load_all(dir.path());
        assert_eq!(list[0].name, "New Name");
    }

    #[test]
    fn update_meta_rejects_empty_name() {
        let dir = tmp();
        let entry = register(dir.path(), "Name", Path::new("/p.scast")).unwrap();
        assert!(update_meta(dir.path(), &entry.id, "").is_err());
        assert!(update_meta(dir.path(), &entry.id, "   ").is_err());
        // Original name unchanged.
        assert_eq!(find(dir.path(), &entry.id).unwrap().name, "Name");
    }

    #[test]
    fn update_meta_errors_on_missing_id() {
        let dir = tmp();
        assert!(update_meta(dir.path(), "ghost", "X").is_err());
    }

    // ── remove ────────────────────────────────────────────────────────────────

    #[test]
    fn remove_deletes_entry() {
        let dir = tmp();
        let a = register(dir.path(), "A", Path::new("/a.scast")).unwrap();
        let b = register(dir.path(), "B", Path::new("/b.scast")).unwrap();
        remove(dir.path(), &a.id).unwrap();
        let list = load_all(dir.path());
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, b.id);
    }

    #[test]
    fn remove_errors_on_missing_id() {
        let dir = tmp();
        assert!(remove(dir.path(), "ghost").is_err());
    }

    #[test]
    fn remove_does_not_affect_other_entries() {
        let dir = tmp();
        let a = register(dir.path(), "A", Path::new("/a.scast")).unwrap();
        let b = register(dir.path(), "B", Path::new("/b.scast")).unwrap();
        let c = register(dir.path(), "C", Path::new("/c.scast")).unwrap();
        remove(dir.path(), &b.id).unwrap();
        let list = load_all(dir.path());
        assert_eq!(list.len(), 2);
        let ids: Vec<_> = list.iter().map(|e| &e.id).collect();
        assert!(ids.contains(&&c.id));
        assert!(ids.contains(&&a.id));
        assert!(!ids.contains(&&b.id));
    }

    // ── crash-safety of the on-disk index ─────────────────────────────────────

    #[test]
    fn a_partial_registry_write_reads_as_empty_total_loss() {
        // The gap atomic writes close: a half-written registry.json (what an
        // interrupted plain fs::write leaves) is unparseable, and load_all maps
        // unparseable → empty, so every project silently disappears.
        let dir = tmp();
        register(dir.path(), "Keepme", Path::new("/k.scast")).unwrap();
        // Simulate a crash mid plain-write: truncate the file to a JSON prefix.
        fs::write(registry_path(dir.path()), b"[{\"id\":\"a\"").unwrap();
        assert!(
            load_all(dir.path()).is_empty(),
            "a corrupt registry is read as empty — the data-loss the fix prevents"
        );
    }

    #[test]
    fn save_all_writes_atomically_with_no_temp_litter() {
        // After a real save the directory holds the registry and nothing else —
        // the temp file used for the atomic rename is gone, and the content is
        // a complete, parseable list (never a partial one).
        let dir = tmp();
        register(dir.path(), "A", Path::new("/a.scast")).unwrap();
        register(dir.path(), "B", Path::new("/b.scast")).unwrap();
        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec![REGISTRY_FILE], "stray temp file left behind");
        assert_eq!(load_all(dir.path()).len(), 2);
    }

    // ── round-trip integration ────────────────────────────────────────────────

    #[test]
    fn full_lifecycle_new_find_update_remove() {
        let dir = tmp();

        // Register two projects.
        let p1 = register(dir.path(), "Sermon Recap", Path::new("/s1.scast")).unwrap();
        let p2 = register(dir.path(), "Youth Podcast", Path::new("/s2.scast")).unwrap();

        // List returns both.
        assert_eq!(load_all(dir.path()).len(), 2);

        // Update name of p1.
        let updated = update_meta(dir.path(), &p1.id, "Sermon Recap 2025").unwrap();
        assert_eq!(updated.name, "Sermon Recap 2025");

        // p2 is unaffected.
        assert_eq!(find(dir.path(), &p2.id).unwrap().name, "Youth Podcast");

        // Delete p2.
        remove(dir.path(), &p2.id).unwrap();
        assert_eq!(load_all(dir.path()).len(), 1);

        // p1 still findable with its new name.
        let remaining = find(dir.path(), &p1.id).unwrap();
        assert_eq!(remaining.name, "Sermon Recap 2025");
    }
}
