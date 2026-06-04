//! The `.scast` project folder, its manifest, the recent-projects list, and
//! database backups.
//!
//! Layout (plan 2.1):
//! ```text
//! project_name.scast/
//! ├── manifest.json     project metadata (this module)
//! ├── project.sqlite    detailed state (the `store` module)
//! ├── takes/            one folder per take, raw WAVs
//! ├── edits/            non-destructive edit decisions
//! ├── exports/          bounced / exported files
//! └── cache/
//!     └── backups/      rotated project.sqlite backups (keep last 5)
//! ```
//! Audio never lives in `project.sqlite` (only metadata), so the database stays
//! small while `takes/` holds the GBs — directly addressing the "huge files"
//! complaint.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::fsutil::atomic_write;
use super::model::RecentProject;
use crate::error::{AppError, AppResult};

pub const MANIFEST_FILE: &str = "manifest.json";
pub const DB_FILE: &str = "project.sqlite";
const SUBDIRS: [&str; 4] = ["takes", "edits", "exports", "cache"];
const RECENT_FILE: &str = "recent.json";
const MAX_RECENT: usize = 12;
const MAX_BACKUPS: usize = 5;

/// On-disk project manifest. `format_version` lets future versions migrate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub name: String,
    pub created_at: f64,
    pub app_version: String,
    pub format_version: u32,
}

pub fn manifest_path(scast_dir: &Path) -> PathBuf {
    scast_dir.join(MANIFEST_FILE)
}

pub fn db_path(scast_dir: &Path) -> PathBuf {
    scast_dir.join(DB_FILE)
}

/// Directory holding a take's WAVs: `<scast>/takes/<take_id>/`.
pub fn take_dir(scast_dir: &Path, take_id: &str) -> PathBuf {
    scast_dir.join("takes").join(take_id)
}

fn backups_dir(scast_dir: &Path) -> PathBuf {
    scast_dir.join("cache").join("backups")
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// Create the `.scast` folder structure and write its manifest. Fails if a
/// manifest already exists (don't clobber a project).
pub fn create_scast(scast_dir: &Path, name: &str) -> AppResult<Manifest> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("project name cannot be empty".into()));
    }
    if manifest_path(scast_dir).exists() {
        return Err(AppError::Validation(format!(
            "a project already exists at {}",
            scast_dir.display()
        )));
    }
    fs::create_dir_all(scast_dir)?;
    for sub in SUBDIRS {
        fs::create_dir_all(scast_dir.join(sub))?;
    }
    fs::create_dir_all(backups_dir(scast_dir))?;

    let manifest = Manifest {
        name: name.to_string(),
        created_at: now_ms(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        format_version: 1,
    };
    write_manifest(scast_dir, &manifest)?;
    Ok(manifest)
}

fn write_manifest(scast_dir: &Path, manifest: &Manifest) -> AppResult<()> {
    // Atomic: a crash mid-write must not leave a truncated manifest, which
    // `read_manifest` would fail to parse — orphaning an otherwise-intact project.
    atomic_write(
        &manifest_path(scast_dir),
        &serde_json::to_vec_pretty(manifest)?,
    )
}

/// Read and validate a project's manifest.
pub fn read_manifest(scast_dir: &Path) -> AppResult<Manifest> {
    let path = manifest_path(scast_dir);
    let bytes = fs::read(&path).map_err(|_| AppError::NotFound {
        entity: "manifest",
        id: scast_dir.display().to_string(),
    })?;
    let manifest: Manifest = serde_json::from_slice(&bytes)?;
    if manifest.format_version > 1 {
        return Err(AppError::Validation(format!(
            "project format {} is newer than this app supports",
            manifest.format_version
        )));
    }
    Ok(manifest)
}

// ── Recent projects (stored app-side, not in any project) ─────────────────────

fn recent_path(config_dir: &Path) -> PathBuf {
    config_dir.join(RECENT_FILE)
}

/// Load the recent-projects list (most-recent first). Missing/corrupt → empty.
pub fn load_recent(config_dir: &Path) -> Vec<RecentProject> {
    match fs::read(recent_path(config_dir)) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Record that a project was just opened: move it to the front, dedupe by path,
/// keep the most recent `MAX_RECENT`.
pub fn push_recent(config_dir: &Path, entry: RecentProject) -> AppResult<()> {
    let mut list = load_recent(config_dir);
    list.retain(|e| e.path != entry.path);
    list.insert(0, entry);
    list.truncate(MAX_RECENT);
    // Atomic: a crash mid-write must leave the previous recent list intact rather
    // than a truncated file `load_recent` would silently read as empty.
    atomic_write(&recent_path(config_dir), &serde_json::to_vec_pretty(&list)?)
}

// ── Backups ───────────────────────────────────────────────────────────────────

/// Copy `project.sqlite` to `cache/backups/<epoch_ms>.sqlite` and prune to the
/// last `MAX_BACKUPS`. Called hourly by the app while a project is open.
pub fn backup_db(scast_dir: &Path) -> AppResult<PathBuf> {
    let src = db_path(scast_dir);
    if !src.exists() {
        return Err(AppError::NotFound {
            entity: "project.sqlite",
            id: scast_dir.display().to_string(),
        });
    }
    let dir = backups_dir(scast_dir);
    fs::create_dir_all(&dir)?;
    // Filenames are epoch-ms stems so prune can sort them chronologically. Two
    // backups within the same millisecond would collide and the second would
    // silently overwrite the first (leaving fewer than MAX_BACKUPS distinct
    // restore points). Bump the stem to the next free millisecond so every
    // backup is a distinct, still chronologically-ordered, restore point.
    let mut stamp = now_ms() as u64;
    let mut dest = dir.join(format!("{stamp}.sqlite"));
    while dest.exists() {
        stamp += 1;
        dest = dir.join(format!("{stamp}.sqlite"));
    }
    fs::copy(&src, &dest)?;
    prune_backups(&dir, MAX_BACKUPS)?;
    Ok(dest)
}

/// Keep only the newest `keep` backups (filenames are epoch-ms, so lexical sort
/// on the numeric stem is chronological). Pure file logic — testable directly.
pub fn prune_backups(backups_dir: &Path, keep: usize) -> AppResult<()> {
    if !backups_dir.exists() {
        return Ok(());
    }
    let mut files: Vec<PathBuf> = fs::read_dir(backups_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "sqlite"))
        .collect();
    // Sort by numeric filename stem ascending (oldest first).
    files.sort_by_key(|p| {
        p.file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0)
    });
    if files.len() > keep {
        for old in &files[..files.len() - keep] {
            let _ = fs::remove_file(old);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_scast_builds_layout_and_manifest() {
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("My Podcast.scast");
        let m = create_scast(&scast, "My Podcast").unwrap();

        assert_eq!(m.name, "My Podcast");
        assert_eq!(m.format_version, 1);
        for sub in ["takes", "edits", "exports", "cache"] {
            assert!(scast.join(sub).is_dir(), "missing {sub}");
        }
        assert!(manifest_path(&scast).exists());
        assert_eq!(read_manifest(&scast).unwrap(), m);
    }

    #[test]
    fn create_scast_refuses_to_clobber() {
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        assert!(create_scast(&scast, "P").is_err());
    }

    #[test]
    fn read_manifest_rejects_future_format() {
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        let future = Manifest {
            name: "P".into(),
            created_at: 0.0,
            app_version: "x".into(),
            format_version: 99,
        };
        write_manifest(&scast, &future).unwrap();
        assert!(read_manifest(&scast).is_err());
    }

    #[test]
    fn read_manifest_rejects_the_very_next_format() {
        // The boundary case: exactly one past the supported max must still bounce,
        // not just an obviously-distant 99.
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        let next = Manifest {
            name: "P".into(),
            created_at: 0.0,
            app_version: "x".into(),
            format_version: 2,
        };
        write_manifest(&scast, &next).unwrap();
        assert!(read_manifest(&scast).is_err());
    }

    #[test]
    fn prune_keeps_newest_and_drops_unparseable_stems_first() {
        // A backup whose stem isn't an epoch parses as 0 → treated as oldest, so it
        // is among the first dropped and the numeric backups survive intact. Prune
        // must not panic on it either.
        let dir = tempfile::tempdir().unwrap();
        for name in ["100.sqlite", "200.sqlite", "300.sqlite", "junk.sqlite"] {
            fs::write(dir.path().join(name), b"x").unwrap();
        }
        // A non-backup file must be left untouched regardless of count.
        fs::write(dir.path().join("notes.txt"), b"x").unwrap();

        prune_backups(dir.path(), 2).unwrap();

        let mut remaining: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        remaining.sort();
        // Newest two epochs kept; junk (stem→0, oldest) dropped; .txt spared.
        assert_eq!(remaining, vec!["200.sqlite", "300.sqlite", "notes.txt"]);
    }

    #[test]
    fn prune_is_a_noop_below_the_limit_and_for_missing_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("100.sqlite"), b"x").unwrap();
        prune_backups(dir.path(), 5).unwrap();
        assert!(dir.path().join("100.sqlite").exists());

        // A directory that doesn't exist yet must succeed silently.
        prune_backups(&dir.path().join("absent"), 5).unwrap();
    }

    #[test]
    fn recent_dedupes_and_orders_most_recent_first() {
        let cfg = tempfile::tempdir().unwrap();
        let mk = |path: &str, t: f64| RecentProject {
            name: path.into(),
            path: path.into(),
            last_opened: t,
        };
        push_recent(cfg.path(), mk("/a.scast", 1.0)).unwrap();
        push_recent(cfg.path(), mk("/b.scast", 2.0)).unwrap();
        push_recent(cfg.path(), mk("/a.scast", 3.0)).unwrap(); // reopen a

        let list = load_recent(cfg.path());
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, "/a.scast"); // most recent first
        assert_eq!(list[1].path, "/b.scast");
    }

    #[test]
    fn backups_prune_to_keep_newest() {
        let dir = tempfile::tempdir().unwrap();
        // Fake backups named by epoch-ms.
        for ms in [100u64, 200, 300, 400, 500, 600, 700] {
            fs::write(dir.path().join(format!("{ms}.sqlite")), b"x").unwrap();
        }
        prune_backups(dir.path(), 5).unwrap();
        let mut remaining: Vec<u64> = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.parse::<u64>().ok())
            })
            .collect();
        remaining.sort_unstable();
        assert_eq!(remaining, vec![300, 400, 500, 600, 700]); // oldest two dropped
    }

    #[test]
    fn backup_db_copies_and_returns_path() {
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        fs::write(db_path(&scast), b"fake-sqlite").unwrap();

        let backup = backup_db(&scast).unwrap();
        assert!(backup.exists());
        assert_eq!(fs::read(&backup).unwrap(), b"fake-sqlite");
    }

    #[test]
    fn rapid_backups_never_collide_or_overwrite() {
        // Two (or more) backups taken within the same millisecond used to share
        // the same `<epoch_ms>.sqlite` name, so the later one silently overwrote
        // the earlier and the retention window held fewer distinct restore
        // points than expected. Each backup must now be a distinct file.
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        fs::write(db_path(&scast), b"fake-sqlite").unwrap();

        let mut paths = std::collections::HashSet::new();
        for _ in 0..4 {
            let b = backup_db(&scast).unwrap();
            assert!(b.exists());
            // A still-numeric stem keeps prune's chronological sort working.
            assert!(b
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .is_some());
            paths.insert(b);
        }
        // All four are distinct files (under MAX_BACKUPS so none pruned).
        assert_eq!(paths.len(), 4, "rapid backups collided");
        let on_disk = fs::read_dir(backups_dir(&scast)).unwrap().count();
        assert_eq!(on_disk, 4);
    }

    #[test]
    fn manifest_write_is_atomic_leaving_no_temp_files() {
        // write_manifest now routes through atomic_write: the scast dir holds the
        // manifest plus the subdirs, never a stray `.manifest.json.*.tmp`.
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("p.scast");
        create_scast(&scast, "P").unwrap();
        let stray: Vec<_> = fs::read_dir(&scast)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp"))
            .collect();
        assert!(stray.is_empty(), "left temp files: {stray:?}");
        assert!(read_manifest(&scast).is_ok());
    }
}
