//! Recording session orchestration: rings → writer thread → WAV, plus meters.
//!
//! This is the part of the engine that can be fully tested WITHOUT audio
//! hardware. The real-time cpal callback (see `stream.rs`) is the only piece
//! that needs a device; everything it does — de-interleave, push to rings,
//! update meters — is exposed as `CaptureSink::push_interleaved`, which the
//! integration test drives directly with synthetic frames.
//!
//! Threads:
//! - the audio callback owns the `CaptureSink` (producers + meters), pushes
//!   samples, never blocks;
//! - a `writer` thread owns the consumers + `MultiTrackWriter`, drains the
//!   rings every few ms and flushes to disk;
//! - the UI holds the `RecordController` (meters reader + shutdown + take dir).
//!
//! Channel→track mapping is 1:1 for now (input channel c → track c); the
//! interface-channel-to-project-track matrix from plan 1.1 lands with the
//! recording UI in Phase 2.2.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rtrb::{Consumer, Producer, RingBuffer};

use super::meters::PeakMeters;
use super::writer::{MultiTrackWriter, TrackSpec};
use crate::error::{AppError, AppResult};

/// How often the writer thread flushes WAV headers for crash safety.
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
/// Idle nap when the rings are empty, so the writer thread doesn't spin.
const IDLE_NAP: Duration = Duration::from_millis(5);

/// Configuration for one recording session.
#[derive(Debug, Clone)]
pub struct RecordConfig {
    /// Directory for this take's per-track WAVs (e.g. `…/takes/<id>`).
    pub take_dir: PathBuf,
    /// One entry per recorded track; length must equal `channels`.
    pub tracks: Vec<TrackSpec>,
    /// Number of interleaved input channels.
    pub channels: usize,
    pub sample_rate: u32,
}

/// The capture endpoint the audio callback owns. Real-time safe: bounded work,
/// no allocation, no locks. Pushes that don't fit a full ring are counted as
/// dropped (an xrun indicator) rather than blocking.
pub struct CaptureSink {
    producers: Vec<Producer<f32>>,
    meters: Arc<PeakMeters>,
    dropped: Arc<AtomicU64>,
    channels: usize,
}

impl CaptureSink {
    /// Feed one block of interleaved input frames (length = frames × channels).
    /// This is exactly what the cpal data callback calls.
    pub fn push_interleaved(&mut self, data: &[f32]) {
        let ch = self.channels;
        if ch == 0 {
            return;
        }
        for c in 0..ch {
            let mut peak = 0.0_f32;
            let mut dropped = 0u64;
            let mut i = c;
            while i < data.len() {
                let s = data[i];
                let a = s.abs();
                if a > peak {
                    peak = a;
                }
                if self.producers[c].push(s).is_err() {
                    dropped += 1;
                }
                i += ch;
            }
            self.meters.observe(c, peak);
            if dropped > 0 {
                self.dropped.fetch_add(dropped, Ordering::Relaxed);
            }
        }
    }

    /// Number of samples dropped due to a full ring (overrun) so far.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// The UI-side handle to a running session.
pub struct RecordController {
    writer_thread: Option<JoinHandle<AppResult<Vec<u64>>>>,
    shutdown: Arc<AtomicBool>,
    meters: Arc<PeakMeters>,
    dropped: Arc<AtomicU64>,
    take_dir: PathBuf,
}

impl RecordController {
    /// Current peak for a channel in dBFS, resetting the held value (UI polls
    /// this ~60fps).
    pub fn meter_dbfs(&self, channel: usize) -> f32 {
        self.meters.take_dbfs(channel)
    }

    /// Samples dropped to overruns so far (0 is healthy).
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    pub fn take_dir(&self) -> &Path {
        &self.take_dir
    }

    /// Stop the session: signal the writer thread, wait for it to drain and
    /// finalise every WAV, and return per-track sample counts.
    pub fn stop(mut self) -> AppResult<Vec<u64>> {
        self.shutdown.store(true, Ordering::Release);
        match self.writer_thread.take() {
            Some(h) => h
                .join()
                .map_err(|_| AppError::Audio("writer thread panicked".into()))?,
            None => Err(AppError::Internal("session already stopped".into())),
        }
    }
}

/// Start a session: build per-channel rings, spawn the writer thread, and hand
/// back the capture sink (for the audio callback) and the controller (for the
/// UI). Does NOT open any audio device — that's `stream::build_capture_stream`.
pub fn start_session(config: RecordConfig) -> AppResult<(CaptureSink, RecordController)> {
    if config.channels == 0 {
        return Err(AppError::Validation("channels must be > 0".into()));
    }
    if config.tracks.len() != config.channels {
        return Err(AppError::Validation(format!(
            "tracks ({}) must match channels ({})",
            config.tracks.len(),
            config.channels
        )));
    }

    let writer = MultiTrackWriter::create(&config.take_dir, &config.tracks, config.sample_rate)?;

    // One ring per channel, sized for ~1 second of audio so a momentary writer
    // stall can't drop samples.
    let capacity = config.sample_rate.max(1) as usize;
    let mut producers = Vec::with_capacity(config.channels);
    let mut consumers: Vec<Consumer<f32>> = Vec::with_capacity(config.channels);
    for _ in 0..config.channels {
        let (p, c) = RingBuffer::<f32>::new(capacity);
        producers.push(p);
        consumers.push(c);
    }

    let meters = Arc::new(PeakMeters::new(config.channels));
    let dropped = Arc::new(AtomicU64::new(0));
    let shutdown = Arc::new(AtomicBool::new(false));

    let writer_thread = spawn_writer(consumers, writer, Arc::clone(&shutdown), config.channels);

    let sink = CaptureSink {
        producers,
        meters: Arc::clone(&meters),
        dropped: Arc::clone(&dropped),
        channels: config.channels,
    };
    let controller = RecordController {
        writer_thread: Some(writer_thread),
        shutdown,
        meters,
        dropped,
        take_dir: config.take_dir,
    };
    Ok((sink, controller))
}

/// The writer thread: drain every ring into the WAVs, flush periodically, and
/// on shutdown do a final full drain before finalising.
fn spawn_writer(
    mut consumers: Vec<Consumer<f32>>,
    mut writer: MultiTrackWriter,
    shutdown: Arc<AtomicBool>,
    channels: usize,
) -> JoinHandle<AppResult<Vec<u64>>> {
    thread::spawn(move || -> AppResult<Vec<u64>> {
        let mut scratch: Vec<f32> = Vec::with_capacity(4096);
        let mut last_flush = Instant::now();

        loop {
            let mut moved = false;
            for (c, consumer) in consumers.iter_mut().enumerate().take(channels) {
                scratch.clear();
                while let Ok(s) = consumer.pop() {
                    scratch.push(s);
                }
                if !scratch.is_empty() {
                    writer.write_block(c, &scratch)?;
                    moved = true;
                }
            }

            if last_flush.elapsed() >= FLUSH_INTERVAL {
                writer.flush()?;
                last_flush = Instant::now();
            }

            if shutdown.load(Ordering::Acquire) {
                // Final drain: capture has stopped, so the rings won't grow.
                for (c, consumer) in consumers.iter_mut().enumerate().take(channels) {
                    scratch.clear();
                    while let Ok(s) = consumer.pop() {
                        scratch.push(s);
                    }
                    if !scratch.is_empty() {
                        writer.write_block(c, &scratch)?;
                    }
                }
                break;
            }

            if !moved {
                thread::sleep(IDLE_NAP);
            }
        }

        writer.finalize()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track_specs(n: usize) -> Vec<TrackSpec> {
        (0..n)
            .map(|i| TrackSpec {
                track_id: format!("track{i}"),
            })
            .collect()
    }

    #[test]
    fn full_pipeline_records_synthetic_audio_to_disk() {
        // This drives the entire ring → writer → WAV path with NO audio device:
        // we play the role of the cpal callback by calling push_interleaved.
        let dir = tempfile::tempdir().unwrap();
        let config = RecordConfig {
            take_dir: dir.path().to_path_buf(),
            tracks: track_specs(2),
            channels: 2,
            sample_rate: 48_000,
        };
        let (mut sink, controller) = start_session(config).unwrap();

        // 4800 stereo frames: ch0 = +0.5, ch1 = -0.25.
        let mut block = Vec::with_capacity(4800 * 2);
        for _ in 0..4800 {
            block.push(0.5);
            block.push(-0.25);
        }
        sink.push_interleaved(&block);

        // Meters reflect the per-channel peaks immediately.
        assert!((controller.meter_dbfs(0) + 6.0206).abs() < 0.05);
        assert!((controller.meter_dbfs(1) + 12.041).abs() < 0.05);

        // Give the writer thread a moment to drain, then stop.
        thread::sleep(Duration::from_millis(60));
        let counts = controller.stop().unwrap();
        assert_eq!(counts, vec![4800, 4800]);

        // Verify track0 landed on disk with the right length and value.
        let r = hound::WavReader::open(dir.path().join("track0.wav")).unwrap();
        assert_eq!(r.len(), 4800);
        let first: i32 = r.into_samples::<i32>().next().unwrap().unwrap();
        assert!((first - 4_194_303).abs() <= 2, "got {first}");
    }

    #[test]
    fn stop_with_no_input_yields_empty_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let config = RecordConfig {
            take_dir: dir.path().to_path_buf(),
            tracks: track_specs(1),
            channels: 1,
            sample_rate: 48_000,
        };
        let (_sink, controller) = start_session(config).unwrap();
        let counts = controller.stop().unwrap();
        assert_eq!(counts, vec![0]);
        // An empty but valid WAV exists.
        assert!(dir.path().join("track0.wav").exists());
    }

    #[test]
    fn rejects_mismatched_track_and_channel_counts() {
        let dir = tempfile::tempdir().unwrap();
        let config = RecordConfig {
            take_dir: dir.path().to_path_buf(),
            tracks: track_specs(3),
            channels: 2,
            sample_rate: 48_000,
        };
        assert!(start_session(config).is_err());
    }
}
