//! Integration tests for resolving the open project's timeline into playback
//! buffers (Phase 1.4).
//!
//! `audio_play_timeline` is a thin Tauri wrapper over
//! `commands::audio::resolve_playback_tracks`, which takes a plain pool + folder
//! and walks the same region-aware path as export (decode → trim/gain/fade per
//! clip → assemble per track). We drive that core directly against a throwaway
//! temp project seeded by `import_takes`, mirroring `tests/edit_commands.rs` — no
//! Tauri IPC, no audio device.

use std::path::{Path, PathBuf};

use sundaystudio_lib::audio::playback::{start_playback, timeline_len};
use sundaystudio_lib::audio::tone;
use sundaystudio_lib::commands::audio::resolve_playback_tracks;
use sundaystudio_lib::commands::edit::import_takes;
use sundaystudio_lib::project;
use sundaystudio_lib::project::store;

/// A fresh `.scast` project at `sample_rate` Hz; the temp dir is cleaned up when
/// the returned guard drops. Mirrors the helper in `edit_commands.rs`.
async fn temp_project(sample_rate: i32) -> (tempfile::TempDir, PathBuf, sqlx::SqlitePool, String) {
    let root = tempfile::tempdir().unwrap();
    let scast = root.path().join("Test.scast");
    let (pool, p) = project::create(&scast, "Test", sample_rate, 1)
        .await
        .unwrap();
    (root, scast, pool, p.id)
}

/// Write a test-tone WAV of `duration_ms` at `rate` Hz into `dir`.
fn write_wav(dir: &Path, name: &str, rate: u32, duration_ms: u32) -> String {
    let path = dir.join(name);
    tone::write_test_tone(&path, rate, 440.0, duration_ms).expect("write tone");
    path.to_string_lossy().into_owned()
}

#[tokio::test]
async fn resolve_errors_when_the_timeline_is_empty() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    // No regions placed yet → resolving for playback is a clean validation error.
    let err = resolve_playback_tracks(&pool, &scast, &pid)
        .await
        .unwrap_err();
    assert!(
        err.to_string().to_lowercase().contains("nothing to play"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn resolve_assembles_one_playback_track_per_imported_file() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let host = write_wav(src.path(), "host.wav", 48_000, 1000); // 1s
    let guest = write_wav(src.path(), "guest.wav", 48_000, 2000); // 2s
    import_takes(&pool, &scast, &pid, 48_000, vec![host, guest])
        .await
        .unwrap();

    let (tracks, rate) = resolve_playback_tracks(&pool, &scast, &pid).await.unwrap();
    assert_eq!(rate, 48_000);
    assert_eq!(tracks.len(), 2, "one playback track per file");

    // Each track's assembled timeline matches its source duration in samples
    // (1s ≈ 48000, 2s ≈ 96000; the tone writer rounds frames, allow a little).
    assert!((tracks[0].timeline.len() as i64 - 48_000).abs() <= 48);
    assert!((tracks[1].timeline.len() as i64 - 96_000).abs() <= 96);

    // The overall playback length is the longest track.
    assert_eq!(timeline_len(&tracks), tracks[1].timeline.len() as u64);

    // The resolved audio is non-silent (the 440 Hz tone) and within range.
    let peak = tracks[0]
        .timeline
        .iter()
        .fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(peak > 0.1 && peak <= 1.0, "peak {peak}");
}

#[tokio::test]
async fn muted_track_resolves_to_silence_but_keeps_its_slot() {
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let a = write_wav(src.path(), "a.wav", 48_000, 1000);
    let b = write_wav(src.path(), "b.wav", 48_000, 1000);
    import_takes(&pool, &scast, &pid, 48_000, vec![a, b])
        .await
        .unwrap();

    // Mute the first track.
    let mut tracks = store::list_tracks(&pool, &pid).await.unwrap();
    tracks[0].mute = true;
    store::update_track(&pool, &tracks[0]).await.unwrap();

    let (resolved, _) = resolve_playback_tracks(&pool, &scast, &pid).await.unwrap();
    // Both tracks still resolve (the muted one keeps its slot so live unmute is a
    // bit flip, not a timeline rebuild) but the muted one is silent.
    assert_eq!(resolved.len(), 2);
    let muted_peak = resolved[0]
        .timeline
        .iter()
        .fold(0.0_f32, |m, &s| m.max(s.abs()));
    let live_peak = resolved[1]
        .timeline
        .iter()
        .fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(
        muted_peak < 1e-6,
        "muted track should be silent: {muted_peak}"
    );
    assert!(
        live_peak > 0.1,
        "unmuted track should carry audio: {live_peak}"
    );
}

#[tokio::test]
async fn resolved_tracks_play_through_the_full_pipeline() {
    // End-to-end with no device: resolve a real imported take, then run it through
    // the render-thread → ring path and confirm we get a non-trivial output.
    let (_root, scast, pool, pid) = temp_project(48_000).await;
    let src = tempfile::tempdir().unwrap();
    let take = write_wav(src.path(), "take.wav", 48_000, 200); // short
    import_takes(&pool, &scast, &pid, 48_000, vec![take])
        .await
        .unwrap();

    let (tracks, rate) = resolve_playback_tracks(&pool, &scast, &pid).await.unwrap();
    let len = timeline_len(&tracks);
    let mut ctl = start_playback(tracks, rate).unwrap();
    ctl.play();

    let mut collected: Vec<f32> = Vec::new();
    let mut scratch = Vec::new();
    for _ in 0..200 {
        ctl.drain_output(&mut scratch);
        collected.extend_from_slice(&scratch);
        if collected.len() as u64 >= len && !ctl.playing() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    ctl.stop().unwrap();

    assert_eq!(collected.len() as u64, len, "played the whole take");
    let peak = collected.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!(peak > 0.1, "playback output should carry the tone: {peak}");
}
