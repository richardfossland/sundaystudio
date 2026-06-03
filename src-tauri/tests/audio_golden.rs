//! Audio output regression test.
//!
//! Guards that generated audio stays correct across refactors. Originally this
//! fingerprinted exact bytes, but that is the WRONG tool for anything derived
//! from floating-point math: `sin`/`tanh`/`powf` differ in the last bit across
//! platforms and compilers, so a byte-exact golden seeded on one OS fails on
//! another (it did — macOS vs Linux CI). See ADR-0009.
//!
//! Instead we assert signal *properties* with tolerances: the right length and
//! format, the expected peak/RMS amplitude, and the expected frequency (via
//! zero-crossing count). DSP effects are tested the same way in `src/dsp`.

use std::f32::consts::SQRT_2;
use std::thread::sleep;
use std::time::Duration;

use sundaystudio_lib::audio::recorder::{start_session, RecordConfig, RecordController, TrackSpec};
use sundaystudio_lib::audio::tone;

/// Count sign changes (zero crossings) in a sample slice.
fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count()
}

/// Build an N-track, N-channel session in a fresh temp take dir.
fn start_test_session(
    channels: usize,
) -> (
    sundaystudio_lib::audio::recorder::CaptureSink,
    RecordController,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let config = RecordConfig {
        take_dir: dir.path().to_path_buf(),
        tracks: (0..channels)
            .map(|i| TrackSpec {
                track_id: format!("track{i}"),
            })
            .collect(),
        channels,
        sample_rate: 48_000,
    };
    let (sink, controller) = start_session(config).unwrap();
    (sink, controller, dir)
}

/// One interleaved block where channel `c` is held at `levels[c]` for `frames`.
/// Commands enqueued on the controller before the next `push_interleaved` take
/// effect at the start of that push (the sink drains the queue first), so a
/// command immediately precedes the block it should affect.
fn interleaved_block(levels: &[f32], frames: usize) -> Vec<f32> {
    let mut block = Vec::with_capacity(frames * levels.len());
    for _ in 0..frames {
        block.extend_from_slice(levels);
    }
    block
}

#[test]
fn test_tone_is_a_clean_440hz_half_scale_sine() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tone.wav");

    let freq = 440.0;
    let sample_rate = 48_000;
    tone::write_test_tone(&path, sample_rate, freq, 1000).expect("tone writes");

    let reader = hound::WavReader::open(&path).expect("reopen WAV");
    let spec = reader.spec();
    assert_eq!(spec.sample_rate, sample_rate);
    assert_eq!(spec.channels, 1);
    assert_eq!(spec.bits_per_sample, 16);
    assert_eq!(reader.len(), sample_rate, "one second of samples");

    // Normalise i16 → f32 in [-1, 1].
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();

    // Amplitude: the tone is written at -6 dBFS (0.5 full scale).
    let peak = samples.iter().fold(0.0_f32, |m, &s| m.max(s.abs()));
    assert!((peak - 0.5).abs() < 0.01, "peak {peak} should be ~0.5");

    // A sine's RMS is peak / √2.
    let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
    assert!(
        (rms - 0.5 / SQRT_2).abs() < 0.01,
        "rms {rms} should be ~{}",
        0.5 / SQRT_2
    );

    // Frequency: a 440 Hz sine over 1 s has 440 cycles → ~880 zero crossings.
    let crossings = zero_crossings(&samples);
    assert!(
        (crossings as i64 - 880).abs() <= 4,
        "got {crossings} zero crossings, expected ~880 for 440 Hz"
    );
}

// ----------------------------------------------------------------------------
// Phase 1.3 — monitoring mixer, driven through the real session pipeline with
// synthetic frames (no audio device), the same harness as the recorder tests.
// ----------------------------------------------------------------------------

#[test]
fn monitor_ring_fills_with_the_mixed_output_when_enabled() {
    // 4 channels at 0.1/0.2/0.3/0.4 → mono mix is their sum (1.0) per frame.
    let (mut sink, mut controller, _dir) = start_test_session(4);
    let frames = 256;
    let block = interleaved_block(&[0.1, 0.2, 0.3, 0.4], frames);

    controller.set_monitoring(true);
    sink.push_interleaved(&block); // command applies, then this block is mixed

    let mut mono = Vec::new();
    let read = controller.drain_monitor(&mut mono);
    assert_eq!(read, frames, "one mono sample per input frame");
    for v in &mono {
        assert!((v - 1.0).abs() < 1e-5, "mixed sum should be 1.0, got {v}");
    }

    let _ = controller.stop();
}

#[test]
fn monitor_does_not_flow_until_enabled_and_stops_when_disabled() {
    let (mut sink, mut controller, _dir) = start_test_session(2);
    let block = interleaved_block(&[0.5, 0.5], 128);

    // Monitoring off by default: pushing audio leaves the monitor ring empty.
    sink.push_interleaved(&block);
    let mut mono = Vec::new();
    assert_eq!(controller.drain_monitor(&mut mono), 0, "no mix while off");

    // Turn it on → the next block flows.
    controller.set_monitoring(true);
    sink.push_interleaved(&block);
    assert_eq!(controller.drain_monitor(&mut mono), 128, "flows once on");

    // Turn it back off → flow stops again.
    controller.set_monitoring(false);
    sink.push_interleaved(&block);
    assert_eq!(controller.drain_monitor(&mut mono), 0, "stops when off");

    let _ = controller.stop();
}

#[test]
fn soft_mute_zeroes_one_track_in_the_monitor_mix() {
    // Tone of 0.25 on all 4 channels: full mix = 1.0; muting track 2 → 0.75.
    let (mut sink, mut controller, _dir) = start_test_session(4);
    let frames = 200;
    let block = interleaved_block(&[0.25, 0.25, 0.25, 0.25], frames);

    controller.set_monitoring(true);
    controller.set_monitor_mute(2, true);
    sink.push_interleaved(&block);

    let mut mono = Vec::new();
    let read = controller.drain_monitor(&mut mono);
    assert_eq!(read, frames);
    for v in &mono {
        assert!(
            (v - 0.75).abs() < 1e-5,
            "track 2 muted → 0.25×3 = 0.75, got {v}"
        );
    }
    assert!(controller.monitor_muted(2));
    assert!(!controller.monitor_muted(0));

    let _ = controller.stop();
}

#[test]
fn mute_math_is_linear_and_capture_meters_are_unaffected() {
    // Property: N armed tracks at level L mix to N·L; muting one → (N−1)·L. The
    // capture path (meters + on-disk WAV) must be untouched by monitor muting.
    let (mut sink, mut controller, dir) = start_test_session(4);
    let frames = 4_800;
    let level = 0.2_f32;
    let block = interleaved_block(&[level; 4], frames);

    controller.set_monitoring(true);
    controller.set_monitor_mute(1, true); // mute one track in the MONITOR only
    sink.push_interleaved(&block);

    // Monitor mix reflects 3 of 4 tracks.
    let mut mono = Vec::new();
    assert_eq!(controller.drain_monitor(&mut mono), frames);
    let expected = 3.0 * level;
    for v in &mono {
        assert!((v - expected).abs() < 1e-5, "got {v}, expected {expected}");
    }

    // Capture meters still read the per-channel peak for EVERY track, including
    // the monitor-muted one (mute is a monitor concept, not a capture one).
    for c in 0..4 {
        let dbfs = controller.meter_dbfs(c);
        let want = 20.0 * level.log10(); // 0.2 → ~ -13.98 dBFS
        assert!(
            (dbfs - want).abs() < 0.05,
            "ch{c} meter {dbfs} should be ~{want}"
        );
    }

    sleep(Duration::from_millis(60));
    let counts = controller.stop().unwrap();
    assert_eq!(
        counts,
        vec![frames as u64; 4],
        "all 4 tracks captured fully"
    );

    // The monitor-muted track 1 still recorded its full signal to disk.
    let r = hound::WavReader::open(dir.path().join("track1.wav")).unwrap();
    assert_eq!(r.len(), frames as u32);
    let first: i32 = r.into_samples::<i32>().next().unwrap().unwrap();
    // 0.2 of 24-bit full scale (8_388_607) ≈ 1_677_721.
    assert!((first - 1_677_721).abs() <= 4, "captured sample {first}");
}
