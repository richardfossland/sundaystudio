//! Integration tests for the project store (Phase 2.1 data model).
//!
//! `project::store` is the tape-model CRUD layer the whole editor stands on —
//! tracks, takes, regions and markers, plus the two snapshot composers the UI
//! loads in one round-trip. The store's own `#[cfg(test)]` block covers the
//! happy paths; this file drives the same functions against throwaway temp
//! databases (no Tauri state, no device, mirroring `edit_commands.rs`) and
//! presses on the edges the editor will lean on: ordering after mutation,
//! foreign-key cascades, not-found errors, and snapshot consistency.

use sundaystudio_lib::project::model::{Marker, Region, Track};
use sundaystudio_lib::project::store;

/// A fresh in-process project database in a temp dir that is dropped (and
/// cleaned up) when the returned guard goes out of scope. Returns the temp
/// root, the open pool, and the created project id.
async fn temp_project() -> (tempfile::TempDir, sqlx::SqlitePool, String) {
    let dir = tempfile::tempdir().unwrap();
    let pool = store::open_pool(&dir.path().join("project.sqlite"))
        .await
        .unwrap();
    let project = store::create_project(&pool, "Test", 48_000, 2)
        .await
        .unwrap();
    (dir, pool, project.id)
}

/// Build a full-take region on `track` (source == target) spanning `start..end`
/// in the take and placed at `position` on the timeline, with default fades.
fn region_at(take_id: &str, track: &str, start: f64, end: f64, position: f64) -> Region {
    Region {
        id: String::new(),
        take_id: take_id.to_string(),
        source_track_id: track.to_string(),
        target_track_id: track.to_string(),
        start_in_take_ms: start,
        end_in_take_ms: end,
        position_in_timeline_ms: position,
        fade_in_ms: 5.0,
        fade_out_ms: 5.0,
        gain_adjust_db: 0.0,
    }
}

// ── Tracks ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn tracks_take_contiguous_positions_in_insertion_order() {
    let (_d, pool, pid) = temp_project().await;
    for n in 0..5 {
        store::add_track(&pool, &pid, &format!("Mic {n}"), "#D4A73A")
            .await
            .unwrap();
    }
    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    assert_eq!(tracks.len(), 5);
    // Property: positions are 0..len, dense and matching insertion order.
    for (i, t) in tracks.iter().enumerate() {
        assert_eq!(t.position, i as i32, "track {i} out of order");
        assert_eq!(t.name, format!("Mic {i}"));
    }
}

#[tokio::test]
async fn duplicate_track_names_are_allowed_with_distinct_ids() {
    let (_d, pool, pid) = temp_project().await;
    // The store does not enforce name uniqueness — two "Host" mics are legal
    // (a co-host setup), each gets its own id and slot.
    let a = store::add_track(&pool, &pid, "Host", "#D4A73A")
        .await
        .unwrap();
    let b = store::add_track(&pool, &pid, "Host", "#D4A73A")
        .await
        .unwrap();
    assert_ne!(a.id, b.id);
    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].name, tracks[1].name);
}

#[tokio::test]
async fn reordering_tracks_via_update_changes_list_order() {
    let (_d, pool, pid) = temp_project().await;
    let first = store::add_track(&pool, &pid, "First", "#D4A73A")
        .await
        .unwrap();
    let second = store::add_track(&pool, &pid, "Second", "#3a7bd4")
        .await
        .unwrap();

    // Swap their positions — the controlled-component pattern writes the whole
    // track, so the editor's drag-reorder is just two updates.
    let mut a = first.clone();
    a.position = 1;
    let mut b = second.clone();
    b.position = 0;
    store::update_track(&pool, &a).await.unwrap();
    store::update_track(&pool, &b).await.unwrap();

    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    assert_eq!(tracks[0].id, second.id, "Second should now sort first");
    assert_eq!(tracks[1].id, first.id);
}

#[tokio::test]
async fn deleting_a_track_is_idempotent_and_does_not_error() {
    let (_d, pool, pid) = temp_project().await;
    let t = store::add_track(&pool, &pid, "Doomed", "#D4A73A")
        .await
        .unwrap();
    store::delete_track(&pool, &t.id).await.unwrap();
    // Deleting an already-gone track is a no-op (SQL DELETE affects 0 rows), not
    // an error — matching the other delete_* helpers.
    store::delete_track(&pool, &t.id).await.unwrap();
    store::delete_track(&pool, "never-existed").await.unwrap();
    assert!(store::list_tracks(&pool, &pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn updating_a_ghost_track_reports_not_found() {
    let (_d, pool, pid) = temp_project().await;
    let ghost = Track {
        id: "ghost".into(),
        project_id: pid.clone(),
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
    let err = store::update_track(&pool, &ghost).await.unwrap_err();
    assert_eq!(err.code(), "not_found", "unexpected error: {err}");
}

#[tokio::test]
async fn full_mixer_state_round_trips_through_update() {
    let (_d, pool, pid) = temp_project().await;
    let t = store::add_track(&pool, &pid, "Lead", "#D4A73A")
        .await
        .unwrap();

    // Exercise every mutable field at once, including the optional channels and
    // the voice preset (migration 0002).
    let mut edited = t.clone();
    edited.name = "Lead Vocal".into();
    edited.color = "#112233".into();
    edited.input_assignment = Some(3);
    edited.output_assignment = Some(1);
    edited.gain_db = -4.5;
    edited.pan = -0.5;
    edited.mute = true;
    edited.solo = true;
    edited.armed = true;
    edited.position = 0;
    edited.voice_preset = Some("broadcast".into());
    store::update_track(&pool, &edited).await.unwrap();

    let reloaded = &store::list_tracks(&pool, &pid).await.unwrap()[0];
    assert_eq!(reloaded, &edited, "every field should persist verbatim");
}

// ── Markers ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn markers_always_list_sorted_by_position_regardless_of_insert_order() {
    let (_d, pool, pid) = temp_project().await;
    // Insert deliberately out of order; the index sorts on read.
    for (ms, label) in [(9000.0, "Outro"), (0.0, "Intro"), (4500.0, "Mid")] {
        store::add_marker(&pool, &pid, ms, label, "#D4A73A")
            .await
            .unwrap();
    }
    let markers = store::list_markers(&pool, &pid).await.unwrap();
    let positions: Vec<f64> = markers.iter().map(|m| m.position_ms).collect();
    assert_eq!(positions, vec![0.0, 4500.0, 9000.0]);
    assert_eq!(markers[0].label, "Intro");
    assert_eq!(markers[2].label, "Outro");
}

#[tokio::test]
async fn markers_are_scoped_to_their_project() {
    // Two projects in separate databases never see each other's markers.
    let (_d1, pool1, pid1) = temp_project().await;
    let (_d2, pool2, pid2) = temp_project().await;
    store::add_marker(&pool1, &pid1, 0.0, "P1", "#D4A73A")
        .await
        .unwrap();
    assert_eq!(store::list_markers(&pool1, &pid1).await.unwrap().len(), 1);
    // pid2 lives in its own file; listing under it returns nothing.
    assert!(store::list_markers(&pool2, &pid2).await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_a_marker_leaves_the_rest_intact() {
    let (_d, pool, pid) = temp_project().await;
    let keep = store::add_marker(&pool, &pid, 0.0, "Keep", "#D4A73A")
        .await
        .unwrap();
    let drop = store::add_marker(&pool, &pid, 1000.0, "Drop", "#D4A73A")
        .await
        .unwrap();
    store::delete_marker(&pool, &drop.id).await.unwrap();
    let markers = store::list_markers(&pool, &pid).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0].id, keep.id);
}

// ── Takes ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn takes_round_trip_an_empty_source_track_list() {
    let (_d, pool, pid) = temp_project().await;
    // A take with no source tracks (degenerate but legal) must survive the JSON
    // round-trip as an empty vec, not a decode failure.
    let take = store::add_take(&pool, &pid, 1000.0, 0.0, &[])
        .await
        .unwrap();
    let takes = store::list_takes(&pool, &pid).await.unwrap();
    assert_eq!(takes.len(), 1);
    assert_eq!(takes[0].id, take.id);
    assert!(takes[0].source_tracks.is_empty());
}

#[tokio::test]
async fn takes_list_oldest_first_by_start_time() {
    let (_d, pool, pid) = temp_project().await;
    // Insert newest-then-oldest; the index orders by started_at ascending.
    store::add_take(&pool, &pid, 5000.0, 100.0, &["t".into()])
        .await
        .unwrap();
    store::add_take(&pool, &pid, 1000.0, 100.0, &["t".into()])
        .await
        .unwrap();
    let starts: Vec<f64> = store::list_takes(&pool, &pid)
        .await
        .unwrap()
        .iter()
        .map(|t| t.started_at)
        .collect();
    assert_eq!(starts, vec![1000.0, 5000.0]);
}

// ── Regions ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn regions_list_per_track_in_timeline_order() {
    let (_d, pool, pid) = temp_project().await;
    let track = store::add_track(&pool, &pid, "T", "#D4A73A").await.unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        9000.0,
        std::slice::from_ref(&track.id),
    )
    .await
    .unwrap();

    // Three clips dropped onto the timeline out of order.
    for pos in [6000.0, 0.0, 3000.0] {
        store::add_region(&pool, region_at(&take.id, &track.id, 0.0, 1000.0, pos))
            .await
            .unwrap();
    }
    let positions: Vec<f64> = store::list_regions(&pool, &track.id)
        .await
        .unwrap()
        .iter()
        .map(|r| r.position_in_timeline_ms)
        .collect();
    assert_eq!(positions, vec![0.0, 3000.0, 6000.0]);
}

#[tokio::test]
async fn add_region_preserves_an_explicit_id_but_fills_a_blank_one() {
    let (_d, pool, pid) = temp_project().await;
    let track = store::add_track(&pool, &pid, "T", "#D4A73A").await.unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        1000.0,
        std::slice::from_ref(&track.id),
    )
    .await
    .unwrap();

    // Blank id → the store mints one (non-empty).
    let minted = store::add_region(&pool, region_at(&take.id, &track.id, 0.0, 500.0, 0.0))
        .await
        .unwrap();
    assert!(!minted.id.is_empty());

    // Explicit id → kept verbatim, so callers (undo/redo) can re-insert a
    // deleted region under its original id.
    let mut explicit = region_at(&take.id, &track.id, 0.0, 500.0, 1000.0);
    explicit.id = "fixed-region-id".into();
    let kept = store::add_region(&pool, explicit).await.unwrap();
    assert_eq!(kept.id, "fixed-region-id");
}

#[tokio::test]
async fn region_can_be_moved_to_a_different_target_track() {
    let (_d, pool, pid) = temp_project().await;
    let from = store::add_track(&pool, &pid, "From", "#D4A73A")
        .await
        .unwrap();
    let to = store::add_track(&pool, &pid, "To", "#3a7bd4")
        .await
        .unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        1000.0,
        std::slice::from_ref(&from.id),
    )
    .await
    .unwrap();
    let r = store::add_region(&pool, region_at(&take.id, &from.id, 0.0, 1000.0, 0.0))
        .await
        .unwrap();

    // Re-target the region (drag a clip from one lane to another).
    let mut moved = r.clone();
    moved.target_track_id = to.id.clone();
    store::update_region(&pool, &moved).await.unwrap();

    assert!(store::list_regions(&pool, &from.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store::list_regions(&pool, &to.id).await.unwrap().len(), 1);
}

#[tokio::test]
async fn updating_a_ghost_region_reports_not_found() {
    let (_d, pool, pid) = temp_project().await;
    let track = store::add_track(&pool, &pid, "T", "#D4A73A").await.unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        1000.0,
        std::slice::from_ref(&track.id),
    )
    .await
    .unwrap();
    let mut ghost = region_at(&take.id, &track.id, 0.0, 1000.0, 0.0);
    ghost.id = "nope".into();
    let err = store::update_region(&pool, &ghost).await.unwrap_err();
    assert_eq!(err.code(), "not_found", "unexpected error: {err}");
}

// ── Cascades ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn deleting_a_take_cascades_to_its_regions_but_spares_the_track() {
    let (_d, pool, pid) = temp_project().await;
    let track = store::add_track(&pool, &pid, "T", "#D4A73A").await.unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        2000.0,
        std::slice::from_ref(&track.id),
    )
    .await
    .unwrap();
    store::add_region(&pool, region_at(&take.id, &track.id, 0.0, 1000.0, 0.0))
        .await
        .unwrap();
    store::add_region(
        &pool,
        region_at(&take.id, &track.id, 1000.0, 2000.0, 1000.0),
    )
    .await
    .unwrap();
    assert_eq!(
        store::list_regions(&pool, &track.id).await.unwrap().len(),
        2
    );

    // Deleting the source take removes both placed clips (FK ON DELETE CASCADE
    // on region.take_id), but the empty track itself remains.
    sqlx::query("DELETE FROM take WHERE id = ?")
        .bind(&take.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(store::list_regions(&pool, &track.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(store::list_tracks(&pool, &pid).await.unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_a_track_only_cascades_regions_on_that_track() {
    let (_d, pool, pid) = temp_project().await;
    let kept = store::add_track(&pool, &pid, "Kept", "#D4A73A")
        .await
        .unwrap();
    let doomed = store::add_track(&pool, &pid, "Doomed", "#3a7bd4")
        .await
        .unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        1000.0,
        &[kept.id.clone(), doomed.id.clone()],
    )
    .await
    .unwrap();
    store::add_region(&pool, region_at(&take.id, &kept.id, 0.0, 1000.0, 0.0))
        .await
        .unwrap();
    store::add_region(&pool, region_at(&take.id, &doomed.id, 0.0, 1000.0, 0.0))
        .await
        .unwrap();

    store::delete_track(&pool, &doomed.id).await.unwrap();

    // The doomed track's region is gone; the kept track's region survives. The
    // take itself is untouched (only its target_track_id link cascaded).
    let surviving = store::list_project_regions(&pool, &pid).await.unwrap();
    assert_eq!(surviving.len(), 1);
    assert_eq!(surviving[0].target_track_id, kept.id);
    assert_eq!(store::list_takes(&pool, &pid).await.unwrap().len(), 1);
}

// ── Snapshots ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn project_region_listing_spans_tracks_in_one_timeline_order() {
    let (_d, pool, pid) = temp_project().await;
    let a = store::add_track(&pool, &pid, "A", "#D4A73A").await.unwrap();
    let b = store::add_track(&pool, &pid, "B", "#3a7bd4").await.unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        9000.0,
        &[a.id.clone(), b.id.clone()],
    )
    .await
    .unwrap();
    // Interleave clips across the two lanes; list_project_regions must merge
    // them into a single timeline-ordered sequence.
    store::add_region(&pool, region_at(&take.id, &a.id, 0.0, 500.0, 4000.0))
        .await
        .unwrap();
    store::add_region(&pool, region_at(&take.id, &b.id, 0.0, 500.0, 1000.0))
        .await
        .unwrap();
    store::add_region(&pool, region_at(&take.id, &a.id, 0.0, 500.0, 7000.0))
        .await
        .unwrap();

    let positions: Vec<f64> = store::list_project_regions(&pool, &pid)
        .await
        .unwrap()
        .iter()
        .map(|r| r.position_in_timeline_ms)
        .collect();
    // Property: globally sorted by timeline position, ignoring which lane.
    assert_eq!(positions, vec![1000.0, 4000.0, 7000.0]);
}

#[tokio::test]
async fn snapshot_and_timeline_compose_consistently() {
    let (_d, pool, pid) = temp_project().await;
    let track = store::add_track(&pool, &pid, "Host", "#D4A73A")
        .await
        .unwrap();
    store::add_marker(&pool, &pid, 0.0, "Intro", "#D4A73A")
        .await
        .unwrap();
    let take = store::add_take(
        &pool,
        &pid,
        store::now_ms(),
        1000.0,
        std::slice::from_ref(&track.id),
    )
    .await
    .unwrap();
    store::add_region(&pool, region_at(&take.id, &track.id, 0.0, 1000.0, 0.0))
        .await
        .unwrap();

    let snap = store::load_snapshot(&pool).await.unwrap();
    let timeline = store::load_timeline(&pool, &pid).await.unwrap();

    // The two composers slice the same project: snapshot owns project/tracks/
    // markers, timeline owns takes/regions — together the whole editor state.
    assert_eq!(snap.project.id, pid);
    assert_eq!(snap.tracks.len(), 1);
    assert_eq!(snap.markers.len(), 1);
    assert_eq!(timeline.takes.len(), 1);
    assert_eq!(timeline.regions.len(), 1);
    // Cross-check: the snapshot track and the timeline region agree on the id.
    assert_eq!(timeline.regions[0].target_track_id, snap.tracks[0].id);
    // And the region's take is one the timeline reports.
    assert_eq!(timeline.regions[0].take_id, timeline.takes[0].id);
}

#[tokio::test]
async fn empty_project_yields_empty_snapshot_and_timeline() {
    let (_d, pool, pid) = temp_project().await;
    let snap = store::load_snapshot(&pool).await.unwrap();
    assert!(snap.tracks.is_empty());
    assert!(snap.markers.is_empty());
    let timeline = store::load_timeline(&pool, &pid).await.unwrap();
    assert!(timeline.takes.is_empty());
    assert!(timeline.regions.is_empty());

    // A no-marker, no-track project still loads its project row cleanly.
    assert_eq!(snap.project.sample_rate, 48_000);
}

// A small property check on marker construction round-tripping every field, so
// a future column addition that forgets a bind shows up here.
#[tokio::test]
async fn marker_fields_round_trip_verbatim() {
    let (_d, pool, pid) = temp_project().await;
    let made = store::add_marker(&pool, &pid, 1234.5, "Chapter 7", "#abcdef")
        .await
        .unwrap();
    let loaded: Marker = store::list_markers(&pool, &pid).await.unwrap().remove(0);
    assert_eq!(loaded, made);
    assert_eq!(loaded.position_ms, 1234.5);
    assert_eq!(loaded.label, "Chapter 7");
    assert_eq!(loaded.color, "#abcdef");
}
