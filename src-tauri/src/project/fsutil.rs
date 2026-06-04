//! Crash-safe filesystem helpers for the small JSON indexes (registry, recent
//! list, project manifest).
//!
//! These files are the only record of *what projects exist* and *how to read
//! them*. They were written with a plain `fs::write`, which truncates the target
//! and then writes — so a crash or power loss between truncate and the final
//! byte leaves a 0-byte or half-written file. The loaders tolerate corruption by
//! returning an empty list, which turns a partial write into silent total data
//! loss: every project vanishes from the home screen even though the `.scast`
//! folders survive on disk.
//!
//! `atomic_write` removes that window: it writes to a temp file in the *same*
//! directory (so the final step is a same-filesystem rename, which is atomic on
//! POSIX and a replace on Windows) and only then renames it over the target. A
//! crash therefore leaves either the complete old file or the complete new one —
//! never a partial one.

use std::fs;
use std::path::Path;

use crate::error::{AppError, AppResult};

/// Write `bytes` to `path` atomically: a crash mid-write leaves the previous
/// file intact rather than a truncated one. The parent directory is created if
/// missing. The temp file is named off the target plus the process id and a
/// nanosecond timestamp so concurrent writers in the same dir don't collide.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let dir = path.parent().ok_or_else(|| {
        AppError::Internal(format!("path has no parent directory: {}", path.display()))
    })?;
    fs::create_dir_all(dir)?;

    let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("index");
    let tmp = dir.join(format!(".{file_name}.{}.tmp", unique_suffix()));

    // Best-effort cleanup of the temp file on any failure before the rename, so a
    // failed write doesn't litter the directory with stale temp files.
    if let Err(e) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    if let Err(e) = fs::rename(&tmp, path) {
        let _ = fs::remove_file(&tmp);
        return Err(e.into());
    }
    Ok(())
}

/// A per-call-unique suffix (pid + monotonic-ish nanos) for the temp filename,
/// so two writers targeting the same file in the same directory don't clobber
/// each other's temp file before either renames.
fn unique_suffix() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", std::process::id(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_the_file_and_parent_dir() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("nested").join("registry.json");
        atomic_write(&target, b"[1,2,3]").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"[1,2,3]");
    }

    #[test]
    fn atomic_write_replaces_existing_content_wholesale() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("registry.json");
        atomic_write(&target, b"old-and-longer-content").unwrap();
        atomic_write(&target, b"new").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn atomic_write_leaves_no_temp_files_behind() {
        // A successful write must rename its temp file away, so the directory
        // only ever holds the target — no `.registry.json.<pid>.tmp` litter.
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("registry.json");
        atomic_write(&target, b"x").unwrap();
        atomic_write(&target, b"y").unwrap();
        let entries: Vec<String> = fs::read_dir(root.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(entries, vec!["registry.json"]);
    }

    #[test]
    fn a_truncated_file_is_what_a_plain_write_crash_would_leave() {
        // Demonstrates the gap atomic_write closes: a half-written JSON file (what
        // `fs::write` interrupted mid-write produces) is unparseable, and the
        // loaders treat unparseable as empty → total silent loss. atomic_write
        // never exposes this partial state because it renames a complete temp file.
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("registry.json");
        // Simulate a crash mid plain-write: only a prefix of the JSON landed.
        fs::write(&target, b"[{\"id\":\"a\",\"name\":").unwrap();
        let parsed: Result<Vec<serde_json::Value>, _> =
            serde_json::from_slice(&fs::read(&target).unwrap());
        assert!(parsed.is_err(), "a partial plain-write is corrupt");

        // An atomic write over it yields a complete, parseable file.
        atomic_write(&target, b"[]").unwrap();
        let parsed2: Vec<serde_json::Value> =
            serde_json::from_slice(&fs::read(&target).unwrap()).unwrap();
        assert!(parsed2.is_empty());
    }
}
