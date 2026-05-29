//! Project file format + persistence (Phase 2.1).
//!
//! A project is a `.scast` folder (`scast` module) whose `project.sqlite`
//! holds the tape-model state (`store` module) typed by `model`. Audio lives in
//! `takes/` as WAVs; the database holds only metadata.
//!
//! This `mod` ties the two together into the two operations the app performs:
//! `create` (new folder + database + project row) and `open` (validate folder,
//! open database, load snapshot). Both are async (SQLite via sqlx) and tested.

pub mod model;
pub mod scast;
pub mod store;
pub mod templates;

use std::path::Path;

use sqlx::SqlitePool;

use crate::error::AppResult;
use model::{Project, ProjectSnapshot};

/// Create a brand-new project: build the `.scast` folder, open its database,
/// run migrations, and insert the project row. Returns the open pool and the
/// created project.
pub async fn create(
    scast_dir: &Path,
    name: &str,
    sample_rate: i32,
    channel_count: i32,
) -> AppResult<(SqlitePool, Project)> {
    scast::create_scast(scast_dir, name)?;
    let pool = store::open_pool(&scast::db_path(scast_dir)).await?;
    let project = store::create_project(&pool, name, sample_rate, channel_count).await?;
    Ok((pool, project))
}

/// Open an existing project: validate its manifest, open the database, and load
/// the full snapshot (project + tracks + markers).
pub async fn open(scast_dir: &Path) -> AppResult<(SqlitePool, ProjectSnapshot)> {
    scast::read_manifest(scast_dir)?;
    let pool = store::open_pool(&scast::db_path(scast_dir)).await?;
    let snapshot = store::load_snapshot(&pool).await?;
    Ok((pool, snapshot))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_then_open_round_trips() {
        let root = tempfile::tempdir().unwrap();
        let scast = root.path().join("Sunday Recap.scast");

        let (pool, project) = create(&scast, "Sunday Recap", 48_000, 2).await.unwrap();
        store::add_track(&pool, &project.id, "Host", "#D4A73A")
            .await
            .unwrap();
        pool.close().await;

        let (_pool2, snap) = open(&scast).await.unwrap();
        assert_eq!(snap.project.name, "Sunday Recap");
        assert_eq!(snap.tracks.len(), 1);
        assert_eq!(snap.tracks[0].name, "Host");
    }

    #[tokio::test]
    async fn open_missing_project_errors() {
        let root = tempfile::tempdir().unwrap();
        assert!(open(&root.path().join("nope.scast")).await.is_err());
    }
}
