//! SQLite-backed project store (sqlx).
//!
//! Every mutation persists immediately — there is no separate "save" step, so
//! the app never shows a "save your work?" prompt (a core promise). All queries
//! are runtime-checked (`sqlx::query`/`query_as`), so no DATABASE_URL or `.sqlx`
//! cache is needed to build.
//!
//! One project per database file. Functions take `&SqlitePool` so they are
//! testable against a throwaway temp database with no app or device.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::model::{Marker, Project, ProjectSnapshot, Region, Take, TimelineSnapshot, Track};
use crate::error::{AppError, AppResult};

/// Epoch milliseconds as f64 (matches the model's time fields).
pub fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

fn new_id() -> String {
    Uuid::now_v7().to_string()
}

/// Open (creating if needed) the SQLite database at `db_path` and run all
/// pending migrations. Foreign keys are enforced.
pub async fn open_pool(db_path: &Path) -> AppResult<SqlitePool> {
    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!().run(&pool).await?;
    Ok(pool)
}

// ── Project ──────────────────────────────────────────────────────────────────

/// Insert the single project row for this database.
pub async fn create_project(
    pool: &SqlitePool,
    name: &str,
    sample_rate: i32,
    channel_count: i32,
) -> AppResult<Project> {
    let project = Project {
        id: new_id(),
        name: name.to_string(),
        sample_rate,
        channel_count,
        created_at: now_ms(),
    };
    sqlx::query(
        "INSERT INTO project (id, name, sample_rate, channel_count, created_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&project.id)
    .bind(&project.name)
    .bind(project.sample_rate)
    .bind(project.channel_count)
    .bind(project.created_at)
    .execute(pool)
    .await?;
    Ok(project)
}

/// Load the project row (one per file).
pub async fn load_project(pool: &SqlitePool) -> AppResult<Project> {
    sqlx::query_as::<_, Project>("SELECT * FROM project LIMIT 1")
        .fetch_optional(pool)
        .await?
        .ok_or(AppError::NotFound {
            entity: "project",
            id: "<only>".to_string(),
        })
}

/// Rename the project.
pub async fn rename_project(pool: &SqlitePool, id: &str, name: &str) -> AppResult<()> {
    let name = name.trim();
    if name.is_empty() {
        return Err(AppError::Validation("project name cannot be empty".into()));
    }
    sqlx::query("UPDATE project SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Track ────────────────────────────────────────────────────────────────────

/// Append a track to the end of the project's track list.
pub async fn add_track(
    pool: &SqlitePool,
    project_id: &str,
    name: &str,
    color: &str,
) -> AppResult<Track> {
    add_track_with(pool, project_id, name, color, None).await
}

/// Append a track, also assigning its interface input channel. Used by
/// templates, which pre-wire mics to channels.
pub async fn add_track_with(
    pool: &SqlitePool,
    project_id: &str,
    name: &str,
    color: &str,
    input_assignment: Option<i32>,
) -> AppResult<Track> {
    // Next position = current count.
    let position: i32 = sqlx::query("SELECT COUNT(*) AS n FROM track WHERE project_id = ?")
        .bind(project_id)
        .fetch_one(pool)
        .await?
        .get::<i64, _>("n") as i32;

    let track = Track {
        id: new_id(),
        project_id: project_id.to_string(),
        name: name.to_string(),
        color: color.to_string(),
        input_assignment,
        output_assignment: None,
        gain_db: 0.0,
        pan: 0.0,
        mute: false,
        solo: false,
        armed: false,
        position,
        voice_preset: None,
    };
    sqlx::query(
        "INSERT INTO track
           (id, project_id, name, color, input_assignment, output_assignment,
            gain_db, pan, mute, solo, armed, position, voice_preset)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&track.id)
    .bind(&track.project_id)
    .bind(&track.name)
    .bind(&track.color)
    .bind(track.input_assignment)
    .bind(track.output_assignment)
    .bind(track.gain_db)
    .bind(track.pan)
    .bind(track.mute)
    .bind(track.solo)
    .bind(track.armed)
    .bind(track.position)
    .bind(&track.voice_preset)
    .execute(pool)
    .await?;
    Ok(track)
}

/// List a project's tracks in display order.
pub async fn list_tracks(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<Track>> {
    Ok(
        sqlx::query_as::<_, Track>("SELECT * FROM track WHERE project_id = ? ORDER BY position")
            .bind(project_id)
            .fetch_all(pool)
            .await?,
    )
}

/// Persist every mutable field of a track (the controlled-component pattern:
/// the UI sends the whole track, we write it).
pub async fn update_track(pool: &SqlitePool, t: &Track) -> AppResult<()> {
    let affected = sqlx::query(
        "UPDATE track SET
           name = ?, color = ?, input_assignment = ?, output_assignment = ?,
           gain_db = ?, pan = ?, mute = ?, solo = ?, armed = ?, position = ?,
           voice_preset = ?
         WHERE id = ?",
    )
    .bind(&t.name)
    .bind(&t.color)
    .bind(t.input_assignment)
    .bind(t.output_assignment)
    .bind(t.gain_db)
    .bind(t.pan)
    .bind(t.mute)
    .bind(t.solo)
    .bind(t.armed)
    .bind(t.position)
    .bind(&t.voice_preset)
    .bind(&t.id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "track",
            id: t.id.clone(),
        });
    }
    Ok(())
}

/// Delete a track (cascades to its regions, effects and automation).
pub async fn delete_track(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM track WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Marker ───────────────────────────────────────────────────────────────────

pub async fn add_marker(
    pool: &SqlitePool,
    project_id: &str,
    position_ms: f64,
    label: &str,
    color: &str,
) -> AppResult<Marker> {
    let marker = Marker {
        id: new_id(),
        project_id: project_id.to_string(),
        position_ms,
        label: label.to_string(),
        color: color.to_string(),
    };
    sqlx::query(
        "INSERT INTO marker (id, project_id, position_ms, label, color)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&marker.id)
    .bind(&marker.project_id)
    .bind(marker.position_ms)
    .bind(&marker.label)
    .bind(&marker.color)
    .execute(pool)
    .await?;
    Ok(marker)
}

pub async fn list_markers(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<Marker>> {
    Ok(sqlx::query_as::<_, Marker>(
        "SELECT * FROM marker WHERE project_id = ? ORDER BY position_ms",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

pub async fn delete_marker(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM marker WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Take & Region ─────────────────────────────────────────────────────────────

/// Record a take. `source_tracks` is stored as a JSON array.
pub async fn add_take(
    pool: &SqlitePool,
    project_id: &str,
    started_at: f64,
    duration_ms: f64,
    source_tracks: &[String],
) -> AppResult<Take> {
    let take = Take {
        id: new_id(),
        project_id: project_id.to_string(),
        started_at,
        duration_ms,
        source_tracks: source_tracks.to_vec(),
    };
    let json = serde_json::to_string(&take.source_tracks)?;
    sqlx::query(
        "INSERT INTO take (id, project_id, started_at, duration_ms, source_tracks)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&take.id)
    .bind(&take.project_id)
    .bind(take.started_at)
    .bind(take.duration_ms)
    .bind(json)
    .execute(pool)
    .await?;
    Ok(take)
}

/// List takes (newest activity first by start time). `source_tracks` is mapped
/// by hand from its JSON column.
pub async fn list_takes(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<Take>> {
    let rows = sqlx::query(
        "SELECT id, project_id, started_at, duration_ms, source_tracks
         FROM take WHERE project_id = ? ORDER BY started_at",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?;

    let mut takes = Vec::with_capacity(rows.len());
    for row in rows {
        let json: String = row.get("source_tracks");
        takes.push(Take {
            id: row.get("id"),
            project_id: row.get("project_id"),
            started_at: row.get("started_at"),
            duration_ms: row.get("duration_ms"),
            source_tracks: serde_json::from_str(&json).unwrap_or_default(),
        });
    }
    Ok(takes)
}

/// Insert a region (generating its id). Regions are the non-destructive edit
/// units; this is what the recorder calls after a take to lay the captured
/// audio onto the timeline 1:1, and what the editor manipulates in Phase 3.
pub async fn add_region(pool: &SqlitePool, mut region: Region) -> AppResult<Region> {
    if region.id.is_empty() {
        region.id = new_id();
    }
    sqlx::query(
        "INSERT INTO region
           (id, take_id, source_track_id, target_track_id, start_in_take_ms,
            end_in_take_ms, position_in_timeline_ms, fade_in_ms, fade_out_ms,
            gain_adjust_db)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&region.id)
    .bind(&region.take_id)
    .bind(&region.source_track_id)
    .bind(&region.target_track_id)
    .bind(region.start_in_take_ms)
    .bind(region.end_in_take_ms)
    .bind(region.position_in_timeline_ms)
    .bind(region.fade_in_ms)
    .bind(region.fade_out_ms)
    .bind(region.gain_adjust_db)
    .execute(pool)
    .await?;
    Ok(region)
}

/// List the regions placed on a track, in timeline order.
pub async fn list_regions(pool: &SqlitePool, target_track_id: &str) -> AppResult<Vec<Region>> {
    Ok(sqlx::query_as::<_, Region>(
        "SELECT * FROM region WHERE target_track_id = ? ORDER BY position_in_timeline_ms",
    )
    .bind(target_track_id)
    .fetch_all(pool)
    .await?)
}

/// All regions in the project, across every track, in timeline order. The editor
/// loads the whole timeline at once; regions reach the project through their
/// target track (`region.target_track_id → track.project_id`).
pub async fn list_project_regions(pool: &SqlitePool, project_id: &str) -> AppResult<Vec<Region>> {
    Ok(sqlx::query_as::<_, Region>(
        "SELECT region.* FROM region
         JOIN track ON region.target_track_id = track.id
         WHERE track.project_id = ?
         ORDER BY region.position_in_timeline_ms",
    )
    .bind(project_id)
    .fetch_all(pool)
    .await?)
}

/// Persist every mutable field of a region (the editor's move/trim/fade/gain
/// edits all flow through here, mirroring `update_track`). The non-destructive
/// promise holds: only the region row changes — the take's WAV is never touched.
pub async fn update_region(pool: &SqlitePool, r: &Region) -> AppResult<()> {
    let affected = sqlx::query(
        "UPDATE region SET
           take_id = ?, source_track_id = ?, target_track_id = ?,
           start_in_take_ms = ?, end_in_take_ms = ?, position_in_timeline_ms = ?,
           fade_in_ms = ?, fade_out_ms = ?, gain_adjust_db = ?
         WHERE id = ?",
    )
    .bind(&r.take_id)
    .bind(&r.source_track_id)
    .bind(&r.target_track_id)
    .bind(r.start_in_take_ms)
    .bind(r.end_in_take_ms)
    .bind(r.position_in_timeline_ms)
    .bind(r.fade_in_ms)
    .bind(r.fade_out_ms)
    .bind(r.gain_adjust_db)
    .bind(&r.id)
    .execute(pool)
    .await?
    .rows_affected();
    if affected == 0 {
        return Err(AppError::NotFound {
            entity: "region",
            id: r.id.clone(),
        });
    }
    Ok(())
}

/// Remove a region (deleting a clip from the timeline; the source take is kept).
pub async fn delete_region(pool: &SqlitePool, id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM region WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Snapshot ──────────────────────────────────────────────────────────────────

/// Load the project plus its tracks and markers in one go.
pub async fn load_snapshot(pool: &SqlitePool) -> AppResult<ProjectSnapshot> {
    let project = load_project(pool).await?;
    let tracks = list_tracks(pool, &project.id).await?;
    let markers = list_markers(pool, &project.id).await?;
    Ok(ProjectSnapshot {
        project,
        tracks,
        markers,
    })
}

/// Load the project's takes and all their placed regions (the editor timeline).
pub async fn load_timeline(pool: &SqlitePool, project_id: &str) -> AppResult<TimelineSnapshot> {
    let takes = list_takes(pool, project_id).await?;
    let regions = list_project_regions(pool, project_id).await?;
    Ok(TimelineSnapshot { takes, regions })
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn temp_pool() -> (tempfile::TempDir, SqlitePool) {
        let dir = tempfile::tempdir().unwrap();
        let pool = open_pool(&dir.path().join("project.sqlite")).await.unwrap();
        (dir, pool)
    }

    #[tokio::test]
    async fn create_and_load_project() {
        let (_d, pool) = temp_pool().await;
        let made = create_project(&pool, "Sunday Recap", 48_000, 2)
            .await
            .unwrap();
        let loaded = load_project(&pool).await.unwrap();
        assert_eq!(made, loaded);
        assert_eq!(loaded.name, "Sunday Recap");
        assert_eq!(loaded.sample_rate, 48_000);
        assert!(loaded.created_at > 0.0);
    }

    #[tokio::test]
    async fn rename_rejects_empty() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "A", 48_000, 2).await.unwrap();
        assert!(rename_project(&pool, &p.id, "   ").await.is_err());
        rename_project(&pool, &p.id, "Renamed").await.unwrap();
        assert_eq!(load_project(&pool).await.unwrap().name, "Renamed");
    }

    #[tokio::test]
    async fn tracks_append_in_order_and_update() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 2).await.unwrap();
        let t0 = add_track(&pool, &p.id, "Host", "#D4A73A").await.unwrap();
        let _t1 = add_track(&pool, &p.id, "Guest", "#3a7bd4").await.unwrap();

        let tracks = list_tracks(&pool, &p.id).await.unwrap();
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].position, 0);
        assert_eq!(tracks[1].position, 1);
        assert_eq!(tracks[0].name, "Host");

        let mut edited = t0.clone();
        edited.mute = true;
        edited.gain_db = -6.0;
        edited.input_assignment = Some(1);
        edited.voice_preset = Some("broadcast".into());
        update_track(&pool, &edited).await.unwrap();

        let reloaded = list_tracks(&pool, &p.id).await.unwrap();
        assert!(reloaded[0].mute);
        assert_eq!(reloaded[0].gain_db, -6.0);
        assert_eq!(reloaded[0].input_assignment, Some(1));
        assert_eq!(reloaded[0].voice_preset.as_deref(), Some("broadcast"));
        // New tracks default to no processing.
        assert_eq!(reloaded[1].voice_preset, None);
    }

    #[tokio::test]
    async fn update_missing_track_errors() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 2).await.unwrap();
        let ghost = Track {
            id: "nope".into(),
            project_id: p.id.clone(),
            name: "x".into(),
            color: "#000".into(),
            input_assignment: None,
            output_assignment: None,
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            armed: false,
            position: 0,
            voice_preset: None,
        };
        assert!(update_track(&pool, &ghost).await.is_err());
    }

    #[tokio::test]
    async fn delete_track_cascades_to_regions() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 1).await.unwrap();
        let track = add_track(&pool, &p.id, "T", "#fff").await.unwrap();
        let take = add_take(
            &pool,
            &p.id,
            now_ms(),
            1000.0,
            std::slice::from_ref(&track.id),
        )
        .await
        .unwrap();
        add_region(
            &pool,
            Region {
                id: String::new(),
                take_id: take.id.clone(),
                source_track_id: track.id.clone(),
                target_track_id: track.id.clone(),
                start_in_take_ms: 0.0,
                end_in_take_ms: 1000.0,
                position_in_timeline_ms: 0.0,
                fade_in_ms: 5.0,
                fade_out_ms: 5.0,
                gain_adjust_db: 0.0,
            },
        )
        .await
        .unwrap();
        assert_eq!(list_regions(&pool, &track.id).await.unwrap().len(), 1);

        delete_track(&pool, &track.id).await.unwrap();
        assert_eq!(list_tracks(&pool, &p.id).await.unwrap().len(), 0);
        // FK ON DELETE CASCADE removed the region too.
        assert_eq!(list_regions(&pool, &track.id).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn takes_round_trip_source_tracks_json() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 2).await.unwrap();
        let srcs = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        add_take(&pool, &p.id, 1000.0, 2500.0, &srcs).await.unwrap();
        let takes = list_takes(&pool, &p.id).await.unwrap();
        assert_eq!(takes.len(), 1);
        assert_eq!(takes[0].source_tracks, srcs);
        assert_eq!(takes[0].duration_ms, 2500.0);
    }

    #[tokio::test]
    async fn markers_sort_by_position() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 2).await.unwrap();
        add_marker(&pool, &p.id, 5000.0, "Chapter 2", "#fff")
            .await
            .unwrap();
        let m1 = add_marker(&pool, &p.id, 1000.0, "Chapter 1", "#fff")
            .await
            .unwrap();
        let markers = list_markers(&pool, &p.id).await.unwrap();
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].label, "Chapter 1");
        delete_marker(&pool, &m1.id).await.unwrap();
        assert_eq!(list_markers(&pool, &p.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn region_update_delete_and_project_listing() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 1).await.unwrap();
        let t0 = add_track(&pool, &p.id, "A", "#fff").await.unwrap();
        let t1 = add_track(&pool, &p.id, "B", "#000").await.unwrap();
        let take = add_take(&pool, &p.id, now_ms(), 2000.0, std::slice::from_ref(&t0.id))
            .await
            .unwrap();

        let r0 = add_region(
            &pool,
            Region {
                id: String::new(),
                take_id: take.id.clone(),
                source_track_id: t0.id.clone(),
                target_track_id: t1.id.clone(),
                start_in_take_ms: 0.0,
                end_in_take_ms: 1000.0,
                position_in_timeline_ms: 500.0,
                fade_in_ms: 5.0,
                fade_out_ms: 5.0,
                gain_adjust_db: 0.0,
            },
        )
        .await
        .unwrap();

        // A region reaches the project through its target track, across tracks.
        let all = list_project_regions(&pool, &p.id).await.unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, r0.id);

        // Move + trim + gain via update.
        let mut edited = r0.clone();
        edited.position_in_timeline_ms = 1200.0;
        edited.end_in_take_ms = 800.0;
        edited.gain_adjust_db = -3.0;
        update_region(&pool, &edited).await.unwrap();
        let reloaded = list_regions(&pool, &t1.id).await.unwrap();
        assert_eq!(reloaded[0].position_in_timeline_ms, 1200.0);
        assert_eq!(reloaded[0].end_in_take_ms, 800.0);
        assert_eq!(reloaded[0].gain_adjust_db, -3.0);

        // Updating a ghost region errors.
        let mut ghost = r0.clone();
        ghost.id = "nope".into();
        assert!(update_region(&pool, &ghost).await.is_err());

        // Delete removes the region but keeps the take.
        delete_region(&pool, &r0.id).await.unwrap();
        assert!(list_project_regions(&pool, &p.id).await.unwrap().is_empty());
        assert_eq!(list_takes(&pool, &p.id).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn timeline_bundles_takes_and_regions() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "P", 48_000, 1).await.unwrap();
        let track = add_track(&pool, &p.id, "T", "#fff").await.unwrap();
        let take = add_take(
            &pool,
            &p.id,
            now_ms(),
            1000.0,
            std::slice::from_ref(&track.id),
        )
        .await
        .unwrap();
        add_region(
            &pool,
            Region {
                id: String::new(),
                take_id: take.id.clone(),
                source_track_id: track.id.clone(),
                target_track_id: track.id.clone(),
                start_in_take_ms: 0.0,
                end_in_take_ms: 1000.0,
                position_in_timeline_ms: 0.0,
                fade_in_ms: 5.0,
                fade_out_ms: 5.0,
                gain_adjust_db: 0.0,
            },
        )
        .await
        .unwrap();
        let timeline = load_timeline(&pool, &p.id).await.unwrap();
        assert_eq!(timeline.takes.len(), 1);
        assert_eq!(timeline.regions.len(), 1);
    }

    #[tokio::test]
    async fn snapshot_bundles_project_tracks_markers() {
        let (_d, pool) = temp_pool().await;
        let p = create_project(&pool, "Snap", 48_000, 2).await.unwrap();
        add_track(&pool, &p.id, "Host", "#fff").await.unwrap();
        add_marker(&pool, &p.id, 0.0, "Intro", "#fff")
            .await
            .unwrap();
        let snap = load_snapshot(&pool).await.unwrap();
        assert_eq!(snap.project.name, "Snap");
        assert_eq!(snap.tracks.len(), 1);
        assert_eq!(snap.markers.len(), 1);
    }
}
