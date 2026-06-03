//! Integration tests for the take-import path (Phase 3.1).
//!
//! `take_import` is, until live multi-track capture is wired on real hardware,
//! the only way audio gets onto the timeline — so its mission-critical logic
//! (file probing, sample-rate validation, track reuse/creation, WAV copy, region
//! placement) deserves end-to-end coverage. The Tauri command is a thin wrapper
//! over `commands::edit::import_takes`, which takes a plain pool + folder, so we
//! drive that core directly against a throwaway temp database and `scast` dir —
//! no Tauri IPC, no device, mirroring the `project::store` unit tests.

use std::path::{Path, PathBuf};

use sundaystudio_lib::audio::tone;
use sundaystudio_lib::commands::edit::{import_takes, probe_wav};
use sundaystudio_lib::project;
use sundaystudio_lib::project::store;

/// A fresh `.scast` project on disk at `sample_rate` Hz, with a temp dir that is
/// dropped (and cleaned up) when the returned guard goes out of scope. Returns
/// the temp root, the project's `scast` dir, the open pool, and the project id.
async fn temp_project(sample_rate: i32) -> (tempfile::TempDir, PathBuf, sqlx::SqlitePool, String) {
    let root = tempfile::tempdir().unwrap();
    let scast = root.path().join("Test.scast");
    let (pool, p) = project::create(&scast, "Test", sample_rate, 1)
        .await
        .unwrap();
    (root, scast, pool, p.id)
}

/// Write a test-tone WAV of `duration_ms` at `rate` Hz into `dir`, returning its
/// path. The tone content is irrelevant here — only its header (rate) and length
/// (duration) drive the import logic under test.
fn write_wav(dir: &Path, name: &str, rate: u32, duration_ms: u32) -> String {
    let path = dir.join(name);
    tone::write_test_tone(&path, rate, 440.0, duration_ms).expect("write tone");
    path.to_string_lossy().into_owned()
}

#[test]
fn probe_wav_reads_rate_and_duration_from_the_header() {
    let dir = tempfile::tempdir().unwrap();
    // 750 ms at 44.1 kHz, and 2 s at 48 kHz — distinct rate + duration each.
    let a = write_wav(dir.path(), "a.wav", 44_100, 750);
    let b = write_wav(dir.path(), "b.wav", 48_000, 2000);

    let (ms_a, rate_a) = probe_wav(Path::new(&a)).unwrap();
    let (ms_b, rate_b) = probe_wav(Path::new(&b)).unwrap();

    // Property 1: probed rate matches the WAV header exactly.
    assert_eq!(rate_a, 44_100);
    assert_eq!(rate_b, 48_000);
    // Duration is frames/rate; the tone writer rounds frames down, so allow 1 ms.
    assert!((ms_a - 750.0).abs() <= 1.0, "ms_a = {ms_a}");
    assert!((ms_b - 2000.0).abs() <= 1.0, "ms_b = {ms_b}");
}

#[tokio::test]
async fn import_rejects_empty_path_list() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let err = import_takes(&pool, &scast, &pid, 48_000, vec![])
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("no files"),
        "unexpected error: {err}"
    );
    // A rejected import leaves the project untouched.
    assert!(store::list_takes(&pool, &pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn import_rejects_cross_rate_wavs_and_stays_a_no_op() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let good = write_wav(src.path(), "good.wav", 48_000, 1000);
    let bad = write_wav(src.path(), "bad.wav", 44_100, 1000);

    let err = import_takes(&pool, &scast, &pid, 48_000, vec![good, bad])
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("44100"), "should name the off-rate: {msg}");
    assert!(msg.contains("48000"), "should name the project rate: {msg}");

    // Probing happens before any mutation, so nothing was created.
    assert!(store::list_takes(&pool, &pid).await.unwrap().is_empty());
    assert!(store::list_tracks(&pool, &pid).await.unwrap().is_empty());
}

#[tokio::test]
async fn import_creates_tracks_takes_and_full_length_regions() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let host = write_wav(src.path(), "host.wav", 48_000, 1500);
    let guest = write_wav(src.path(), "guest.wav", 48_000, 3000);

    let timeline = import_takes(&pool, &scast, &pid, 48_000, vec![host, guest])
        .await
        .unwrap();

    // One take, two regions (one per file).
    assert_eq!(timeline.takes.len(), 1);
    assert_eq!(timeline.regions.len(), 2);

    // Two new tracks were auto-created, named after the file stems.
    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    assert_eq!(tracks.len(), 2);
    assert_eq!(tracks[0].name, "host");
    assert_eq!(tracks[1].name, "guest");

    let take = &timeline.takes[0];
    // Property 5: the take duration is the max of the imported file durations.
    assert!(
        (take.duration_ms - 3000.0).abs() <= 1.0,
        "take.duration_ms = {}",
        take.duration_ms
    );
    // The take's source tracks are exactly the import targets, in order.
    assert_eq!(
        take.source_tracks,
        vec![tracks[0].id.clone(), tracks[1].id.clone()]
    );

    let valid_tracks: Vec<&str> = tracks.iter().map(|t| t.id.as_str()).collect();
    for region in &timeline.regions {
        // Property 4: every region references the take and valid tracks.
        assert_eq!(region.take_id, take.id);
        assert!(valid_tracks.contains(&region.source_track_id.as_str()));
        assert!(valid_tracks.contains(&region.target_track_id.as_str()));
        // Imported regions sit at the timeline origin with the default fades.
        assert_eq!(region.position_in_timeline_ms, 0.0);
        assert_eq!(region.start_in_take_ms, 0.0);
        // Property 6: default anti-click fades are 5 ms in and out.
        assert_eq!(region.fade_in_ms, 5.0);
        assert_eq!(region.fade_out_ms, 5.0);
        assert_eq!(region.gain_adjust_db, 0.0);

        // Property 2: the clip spans the full take — its length equals the
        // matching file's probed duration, and equals end − start.
        let track = tracks
            .iter()
            .find(|t| t.id == region.target_track_id)
            .unwrap();
        let expected_ms = match track.name.as_str() {
            "host" => 1500.0,
            "guest" => 3000.0,
            other => panic!("unexpected track {other}"),
        };
        let span = region.end_in_take_ms - region.start_in_take_ms;
        assert!(
            (span - expected_ms).abs() <= 1.0,
            "span {span} vs {expected_ms}"
        );
    }

    // The WAVs were copied into the take folder under their target-track id.
    let take_dir = scast.join("takes").join(&take.id);
    for track in &tracks {
        assert!(
            take_dir.join(format!("{}.wav", track.id)).exists(),
            "missing copied WAV for track {}",
            track.id
        );
    }
}

#[tokio::test]
async fn import_reuses_existing_tracks_before_creating_new_ones() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    // Pre-seed one track; the first imported file should land on it, the second
    // overflows into a freshly-created track.
    let existing = store::add_track(&pool, &pid, "Existing", "#000000")
        .await
        .unwrap();
    let src = tempfile::tempdir().unwrap();
    let one = write_wav(src.path(), "one.wav", 48_000, 1000);
    let two = write_wav(src.path(), "two.wav", 48_000, 1000);

    import_takes(&pool, &scast, &pid, 48_000, vec![one, two])
        .await
        .unwrap();

    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    // Property 3: track count never exceeds existing + imported files.
    assert_eq!(tracks.len(), 2);
    // The pre-existing track was reused (not renamed); only the overflow is new.
    assert_eq!(tracks[0].id, existing.id);
    assert_eq!(tracks[0].name, "Existing");
    assert_eq!(tracks[1].name, "two");

    // The reused track now carries a region from the import.
    assert_eq!(
        store::list_regions(&pool, &existing.id)
            .await
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn repeated_imports_accumulate_takes_and_keep_track_count_bounded() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let first = write_wav(src.path(), "first.wav", 48_000, 1000);
    let second = write_wav(src.path(), "second.wav", 48_000, 1000);

    // Two single-file imports: the second reuses the track the first created.
    import_takes(&pool, &scast, &pid, 48_000, vec![first])
        .await
        .unwrap();
    let timeline = import_takes(&pool, &scast, &pid, 48_000, vec![second])
        .await
        .unwrap();

    // Two takes accumulated, one region each.
    assert_eq!(timeline.takes.len(), 2);
    assert_eq!(timeline.regions.len(), 2);

    let tracks = store::list_tracks(&pool, &pid).await.unwrap();
    // Property 3: the second import reused track 0 rather than adding a track.
    assert_eq!(tracks.len(), 1);

    // Every region still references a valid take + track.
    let take_ids: Vec<&str> = timeline.takes.iter().map(|t| t.id.as_str()).collect();
    for region in &timeline.regions {
        assert!(take_ids.contains(&region.take_id.as_str()));
        assert_eq!(region.target_track_id, tracks[0].id);
    }
}

#[tokio::test]
async fn import_errors_when_a_source_wav_is_missing() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    // probe_wav opens the file first, so a missing path fails fast (before any
    // copy) with an audio error — the project is left untouched.
    let missing = scast
        .join("does-not-exist.wav")
        .to_string_lossy()
        .into_owned();
    let err = import_takes(&pool, &scast, &pid, 48_000, vec![missing])
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("open"),
        "unexpected error: {err}"
    );
    assert!(store::list_takes(&pool, &pid).await.unwrap().is_empty());
    assert!(store::list_tracks(&pool, &pid).await.unwrap().is_empty());
}
