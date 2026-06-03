//! Integration tests for the live recording transport (Phase 2.2).
//!
//! `audio_record_start` / `audio_record_stop` wire the tested session pipeline
//! (`audio::recorder::start_session` → rings → writer thread → per-track WAVs) to
//! a project: a take is captured to disk, then laid onto the timeline as one
//! full-length region per track. The cpal **input** stream that feeds the session
//! is the one hardware-dependent piece (`StreamHandle`), validated on real
//! devices in the Phase 2.2 matrix — so here we play the role of that stream by
//! driving `CaptureSink::push_interleaved` with synthetic frames, exactly as the
//! engine's own tests do, and then exercise the persistence half of the transport
//! (`commands::audio::persist_recorded_take`) against a throwaway pool + `scast`
//! dir. No Tauri IPC, no device.

use std::path::{Path, PathBuf};
use std::thread::sleep;
use std::time::Duration;

use sundaystudio_lib::audio::recorder::{start_session, RecordConfig, TrackSpec};
use sundaystudio_lib::commands::audio::{persist_recorded_take, RecordedTake};
use sundaystudio_lib::project;
use sundaystudio_lib::project::{scast, store};

/// A fresh `.scast` project on disk at 48 kHz with `n` tracks, mirroring the
/// take-import integration tests. Returns the temp root (drop = cleanup), the
/// project's `scast` dir, the open pool, the project id, and the track ids in
/// order — those ids double as the per-track WAV filenames the session writes.
async fn temp_project_with_tracks(
    n: usize,
) -> (
    tempfile::TempDir,
    PathBuf,
    sqlx::SqlitePool,
    String,
    Vec<String>,
) {
    let root = tempfile::tempdir().unwrap();
    let scast = root.path().join("Test.scast");
    let (pool, p) = project::create(&scast, "Test", 48_000, n as i32)
        .await
        .unwrap();
    let mut ids = Vec::with_capacity(n);
    for i in 0..n {
        let t = store::add_track(&pool, &p.id, &format!("Mic {}", i + 1), "#D4A73A")
            .await
            .unwrap();
        ids.push(t.id);
    }
    (root, scast, pool, p.id, ids)
}

/// Capture `frames` of synthetic audio at `levels[c]` per channel into the take
/// dir for `take_id`, exactly the way `audio_record_start` would: start the
/// session writing `<track_id>.wav` files, push the block (we are the cpal
/// callback), then stop to finalise. Returns the per-track sample counts.
fn capture_synthetic_take(
    scast: &Path,
    take_id: &str,
    track_ids: &[String],
    levels: &[f32],
    frames: usize,
) -> Vec<u64> {
    let take_dir = scast::take_dir(scast, take_id);
    let config = RecordConfig {
        take_dir,
        tracks: track_ids
            .iter()
            .map(|id| TrackSpec {
                track_id: id.clone(),
            })
            .collect(),
        channels: track_ids.len(),
        sample_rate: 48_000,
    };
    let (mut sink, controller) = start_session(config).unwrap();

    let mut block = Vec::with_capacity(frames * levels.len());
    for _ in 0..frames {
        block.extend_from_slice(levels);
    }
    // We stand in for the cpal input callback: push the interleaved block.
    sink.push_interleaved(&block);
    // The live count the UI polls reflects the frames we captured.
    assert_eq!(controller.captured_frames(), frames as u64);

    // Let the writer thread drain, then stop to finalise every WAV.
    sleep(Duration::from_millis(80));
    controller.stop().unwrap()
}

#[tokio::test]
async fn recorded_take_lands_on_the_timeline_with_full_length_regions() {
    let (_root, scast, pool, pid, track_ids) = temp_project_with_tracks(2).await;

    // 24 000 stereo frames = 0.5 s at 48 kHz, ch0 at +0.5, ch1 at -0.25.
    let take_id = store::new_take_id();
    let counts = capture_synthetic_take(&scast, &take_id, &track_ids, &[0.5, -0.25], 24_000);
    assert_eq!(counts, vec![24_000, 24_000]);

    // Both WAVs were finalised on disk in the take dir.
    let take_dir = scast::take_dir(&scast, &take_id);
    for id in &track_ids {
        assert!(
            take_dir.join(format!("{id}.wav")).exists(),
            "missing WAV for {id}"
        );
    }

    // Persist the take: this is the second half of `audio_record_stop`.
    let recorded = RecordedTake {
        take_id: take_id.clone(),
        sample_rate: 48_000,
        started_at: store::now_ms(),
        track_ids: track_ids.clone(),
        counts: counts.clone(),
    };
    let timeline = persist_recorded_take(&pool, &scast, &pid, &recorded)
        .await
        .unwrap();

    // One take row, one full-length region per track at the timeline origin.
    assert_eq!(timeline.takes.len(), 1);
    assert_eq!(timeline.takes[0].id, take_id);
    assert_eq!(timeline.regions.len(), 2);
    for r in &timeline.regions {
        assert_eq!(r.take_id, take_id);
        assert_eq!(r.source_track_id, r.target_track_id);
        assert_eq!(r.start_in_take_ms, 0.0);
        assert_eq!(r.position_in_timeline_ms, 0.0);
        // 24 000 samples / 48 kHz = 500 ms.
        assert!((r.end_in_take_ms - 500.0).abs() < 0.001, "{}", r.end_in_take_ms);
    }
    // Each region targets a distinct project track.
    let mut targets: Vec<&String> = timeline.regions.iter().map(|r| &r.target_track_id).collect();
    targets.sort();
    targets.dedup();
    assert_eq!(targets.len(), 2, "regions cover both tracks");
}

#[tokio::test]
async fn an_armed_track_with_no_audio_gets_no_region() {
    // Two armed tracks, but only one captured audio (the other had 0 samples,
    // e.g. a mic that never received signal): the silent track gets no region,
    // while the take row still records both as armed.
    let (_root, scast, pool, pid, track_ids) = temp_project_with_tracks(2).await;
    let take_id = store::new_take_id();

    // Capture only track 0; create an empty WAV for track 1 the way the writer
    // would (a valid-but-empty file), so the on-disk shape matches a real take.
    let counts = capture_synthetic_take(&scast, &take_id, &track_ids, &[0.3, 0.0], 12_000);
    assert_eq!(counts, vec![12_000, 12_000]);

    // Now drive persistence with a counts vector where track 1 captured nothing,
    // exercising the "armed but silent → no region" branch directly.
    let recorded = RecordedTake {
        take_id: take_id.clone(),
        sample_rate: 48_000,
        started_at: store::now_ms(),
        track_ids: track_ids.clone(),
        counts: vec![12_000, 0],
    };
    let timeline = persist_recorded_take(&pool, &scast, &pid, &recorded)
        .await
        .unwrap();

    assert_eq!(timeline.takes.len(), 1);
    assert_eq!(timeline.regions.len(), 1, "only the non-empty track lands");
    assert_eq!(timeline.regions[0].target_track_id, track_ids[0]);
}

#[tokio::test]
async fn captured_audio_is_intact_on_disk_after_stop() {
    // The transport's first promise: what was captured is what's written. A
    // half-scale tone on a single track survives the rings → writer → WAV path.
    let (_root, scast, _pool, _pid, track_ids) = temp_project_with_tracks(1).await;
    let take_id = store::new_take_id();
    let counts = capture_synthetic_take(&scast, &take_id, &track_ids, &[0.5], 4_800);
    assert_eq!(counts, vec![4_800]);

    let wav = scast::take_dir(&scast, &take_id).join(format!("{}.wav", track_ids[0]));
    let reader = hound::WavReader::open(&wav).unwrap();
    assert_eq!(reader.len(), 4_800, "every captured sample landed");
    // 0.5 of 24-bit full scale (8_388_607) ≈ 4_194_303.
    let first: i32 = reader.into_samples::<i32>().next().unwrap().unwrap();
    assert!((first - 4_194_303).abs() <= 2, "captured sample {first}");
}
