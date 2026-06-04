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

use super::command::{command_channel, CommandRx, CommandTx, RecorderCommand};
use super::meters::PeakMeters;
use super::monitor::{mix_monitor_block, MonitorState};
use super::writer::{MultiTrackWriter, TrackSpec};
use crate::error::{AppError, AppResult};

/// How often the writer thread flushes WAV headers for crash safety.
const FLUSH_INTERVAL: Duration = Duration::from_millis(250);
/// Idle nap when the rings are empty, so the writer thread doesn't spin.
const IDLE_NAP: Duration = Duration::from_millis(5);
/// Room for pending UI→audio control commands (monitor/mute). The audio thread
/// drains the whole queue each block, so it never backs up in practice.
const COMMAND_CAPACITY: usize = 64;

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
///
/// Besides de-interleaving into the per-channel capture rings, the sink also
/// (a) drains the UI→audio command queue once per block to apply monitor/mute
/// changes, and (b) when monitoring is enabled, mixes the armed channels into a
/// mono `monitor` ring the output callback drains to the headphones (Phase 1.3).
pub struct CaptureSink {
    producers: Vec<Producer<f32>>,
    meters: Arc<PeakMeters>,
    dropped: Arc<AtomicU64>,
    /// Running count of frames pushed (one per interleaved frame, i.e. per
    /// per-channel sample). The UI polls this through the controller to show the
    /// take's live duration; real-time safe (one relaxed add per block).
    captured_frames: Arc<AtomicU64>,
    channels: usize,
    /// Control commands from the UI (monitor on/off, per-track mute).
    commands: CommandRx,
    /// Shared monitor control surface (enabled flag + mute mask).
    monitor: Arc<MonitorState>,
    /// Mono mix destination ring for the output (monitoring) callback.
    monitor_ring: Producer<f32>,
    /// Reused scratch for the mono mix, so the real-time path never allocates.
    monitor_scratch: Vec<f32>,
}

impl CaptureSink {
    /// Apply any queued control commands. Drains the whole queue (bounded by the
    /// command-ring capacity) so a burst of UI toggles all land before this
    /// block's audio is processed. Real-time safe: no allocation, no blocking.
    /// `Stop` is observed here but acted on by the session-teardown flag.
    fn drain_commands(&mut self) {
        while let Some(cmd) = self.commands.try_recv() {
            match cmd {
                RecorderCommand::SetMonitoring(on) => self.monitor.set_enabled(on),
                RecorderCommand::SetMute { track, muted } => self.monitor.set_muted(track, muted),
                RecorderCommand::Stop => { /* teardown is driven by the shutdown flag */ }
            }
        }
    }

    /// Feed one block of interleaved input frames (length = frames × channels).
    /// This is exactly what the cpal data callback calls.
    pub fn push_interleaved(&mut self, data: &[f32]) {
        self.drain_commands();

        let ch = self.channels;
        if ch == 0 {
            return;
        }
        // One frame = one sample per channel; count frames so the live duration
        // is channel-count-independent.
        self.captured_frames
            .fetch_add((data.len() / ch) as u64, Ordering::Relaxed);
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

        // Monitor mix: only when enabled, so a player who isn't listening costs
        // nothing and we never feed an output device they haven't opted into.
        if self.monitor.enabled() {
            mix_monitor_block(data, ch, &self.monitor, &mut self.monitor_scratch);
            for &s in &self.monitor_scratch {
                // Overruns on the monitor ring are benign (a momentary glitch in
                // what you HEAR, not in what's recorded), so they're dropped
                // silently rather than counted against capture health.
                let _ = self.monitor_ring.push(s);
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
    /// Set true by the writer thread iff it returned *prematurely* — i.e. a disk
    /// I/O error made it exit before `stop` requested shutdown. The cpal callback
    /// keeps filling the rings (and counting overruns) when this happens, so the
    /// take looks healthy from the meters alone; this flag is the only live signal
    /// that disk writes have stopped, so the UI can warn immediately instead of
    /// discovering the loss only when the user finally presses Stop.
    writer_failed: Arc<AtomicBool>,
    /// Test-only fault injection: when set true the writer thread returns an error
    /// on its next loop iteration, standing in for a real disk failure (disk full,
    /// take dir becomes unwritable) that can't be induced deterministically and
    /// without hardware otherwise.
    force_writer_fail: Arc<AtomicBool>,
    meters: Arc<PeakMeters>,
    dropped: Arc<AtomicU64>,
    /// Live count of captured frames (shared with the sink), so the UI can show
    /// the take's duration while recording without waiting for `stop`.
    captured_frames: Arc<AtomicU64>,
    take_dir: PathBuf,
    /// UI→audio control queue (monitor/mute); the audio thread drains it.
    commands: CommandTx,
    /// Shared monitor control surface, also queryable directly for the UI.
    monitor: Arc<MonitorState>,
    /// Consumer of the mono monitor mix. In production the output (monitoring)
    /// callback owns this; tests drain it to assert the mix.
    monitor_ring: Consumer<f32>,
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

    /// Has the writer thread died prematurely (a disk write error before stop)?
    /// `true` means capture is still running but nothing is reaching disk — the
    /// take is silently being lost and the UI must warn the user now. `false` is
    /// the healthy state (writer draining normally, or already cleanly stopped).
    pub fn writer_failed(&self) -> bool {
        self.writer_failed.load(Ordering::Acquire)
    }

    /// Test-only: arm the writer's fault injection so it returns an error (and
    /// thus sets `writer_failed`) on its next iteration, simulating a disk
    /// failure mid-take. Not part of the production transport.
    #[doc(hidden)]
    pub fn force_writer_fail_for_test(&self) {
        self.force_writer_fail.store(true, Ordering::Release);
    }

    /// Frames captured so far (one per per-channel sample). The UI multiplies by
    /// the sample period for the live take duration.
    pub fn captured_frames(&self) -> u64 {
        self.captured_frames.load(Ordering::Relaxed)
    }

    pub fn take_dir(&self) -> &Path {
        &self.take_dir
    }

    /// Enqueue a control command for the audio thread (non-blocking). Returns
    /// false if the command queue is momentarily full (the UI can retry).
    pub fn send_command(&mut self, cmd: RecorderCommand) -> bool {
        self.commands.send(cmd)
    }

    /// Turn software monitoring on/off via the command queue.
    pub fn set_monitoring(&mut self, on: bool) -> bool {
        self.send_command(RecorderCommand::SetMonitoring(on))
    }

    /// Mute/unmute a track in the monitor mix via the command queue (capture is
    /// unaffected).
    pub fn set_monitor_mute(&mut self, track: usize, muted: bool) -> bool {
        self.send_command(RecorderCommand::SetMute { track, muted })
    }

    /// Is software monitoring currently on? Reflects the last command the audio
    /// thread has processed.
    pub fn monitoring_enabled(&self) -> bool {
        self.monitor.enabled()
    }

    /// Is `track` muted in the monitor mix?
    pub fn monitor_muted(&self, track: usize) -> bool {
        self.monitor.is_muted(track)
    }

    /// Pop the next mono monitor sample, if any. The output callback drains this
    /// to the headphones; tests use it to assert the monitor mix. Returns `None`
    /// when the ring is empty (monitoring off, or already drained).
    pub fn pop_monitor_sample(&mut self) -> Option<f32> {
        self.monitor_ring.pop().ok()
    }

    /// Drain all currently-available monitor samples into `out` (cleared first),
    /// returning how many were read. A test/UI convenience over
    /// `pop_monitor_sample`.
    pub fn drain_monitor(&mut self, out: &mut Vec<f32>) -> usize {
        out.clear();
        while let Ok(s) = self.monitor_ring.pop() {
            out.push(s);
        }
        out.len()
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
    let captured_frames = Arc::new(AtomicU64::new(0));
    let shutdown = Arc::new(AtomicBool::new(false));
    let writer_failed = Arc::new(AtomicBool::new(false));
    let force_writer_fail = Arc::new(AtomicBool::new(false));

    // UI→audio command queue (monitor/mute) and the shared monitor surface.
    let (command_tx, command_rx) = command_channel(COMMAND_CAPACITY);
    let monitor = Arc::new(MonitorState::new());

    // The mono monitor ring, sized like the capture rings (~1 s) so a momentary
    // output-callback stall doesn't lose the live mix.
    let (monitor_producer, monitor_consumer) = RingBuffer::<f32>::new(capacity);

    let writer_thread = spawn_writer(
        consumers,
        writer,
        Arc::clone(&shutdown),
        Arc::clone(&writer_failed),
        Arc::clone(&force_writer_fail),
        config.channels,
    );

    let sink = CaptureSink {
        producers,
        meters: Arc::clone(&meters),
        dropped: Arc::clone(&dropped),
        captured_frames: Arc::clone(&captured_frames),
        channels: config.channels,
        commands: command_rx,
        monitor: Arc::clone(&monitor),
        monitor_ring: monitor_producer,
        monitor_scratch: Vec::with_capacity(capacity),
    };
    let controller = RecordController {
        writer_thread: Some(writer_thread),
        shutdown,
        writer_failed,
        force_writer_fail,
        meters,
        dropped,
        captured_frames,
        take_dir: config.take_dir,
        commands: command_tx,
        monitor,
        monitor_ring: monitor_consumer,
    };
    Ok((sink, controller))
}

/// The writer thread: drain every ring into the WAVs, flush periodically, and
/// on shutdown do a final full drain before finalising.
fn spawn_writer(
    consumers: Vec<Consumer<f32>>,
    writer: MultiTrackWriter,
    shutdown: Arc<AtomicBool>,
    writer_failed: Arc<AtomicBool>,
    force_writer_fail: Arc<AtomicBool>,
    channels: usize,
) -> JoinHandle<AppResult<Vec<u64>>> {
    thread::spawn(move || -> AppResult<Vec<u64>> {
        // Run the drain/flush loop; on ANY error return, mark the writer as
        // failed so the UI can see — via the controller / RecordingStatus — that
        // disk writes stopped even though the cpal callback keeps capturing. We
        // only flag *premature* failure: a clean stop never sets the flag.
        let result = writer_loop(consumers, writer, &shutdown, &force_writer_fail, channels);
        if result.is_err() {
            writer_failed.store(true, Ordering::Release);
        }
        result
    })
}

/// The writer drain/flush/finalise loop, split out so the spawn wrapper can
/// observe its `Result` and set the shared `writer_failed` flag on early exit.
fn writer_loop(
    mut consumers: Vec<Consumer<f32>>,
    mut writer: MultiTrackWriter,
    shutdown: &AtomicBool,
    force_writer_fail: &AtomicBool,
    channels: usize,
) -> AppResult<Vec<u64>> {
    let mut scratch: Vec<f32> = Vec::with_capacity(4096);
    let mut last_flush = Instant::now();

    loop {
        // Test-only injected fault: stand in for a real disk write error so the
        // premature-exit path is exercised deterministically without hardware.
        if force_writer_fail.load(Ordering::Acquire) {
            return Err(AppError::Audio("simulated writer disk failure".into()));
        }

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
    fn writer_death_mid_take_is_visible_while_capture_continues() {
        // Regression: a disk write error mid-recording makes the writer thread
        // exit, but the cpal callback keeps pushing into the rings. Before the
        // fix the controller had no live signal of this, so the UI showed a
        // healthy take and the loss only surfaced at stop(). Now writer_failed()
        // flips true while capture is still running.
        let dir = tempfile::tempdir().unwrap();
        let config = RecordConfig {
            take_dir: dir.path().to_path_buf(),
            tracks: track_specs(1),
            channels: 1,
            sample_rate: 48_000,
        };
        let (mut sink, controller) = start_session(config).unwrap();

        // Healthy at the start.
        assert!(!controller.writer_failed());

        // Induce the writer's disk failure, then keep capturing (we are the cpal
        // callback): pushes still succeed and frames still count up.
        controller.force_writer_fail_for_test();
        // Give the writer thread time to hit the injected fault and exit.
        for _ in 0..50 {
            if controller.writer_failed() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        // The take is still "rolling" from the capture side...
        sink.push_interleaved(&[0.5; 480]);
        assert_eq!(controller.captured_frames(), 480);
        // ...but the writer is dead and the controller surfaces it.
        assert!(
            controller.writer_failed(),
            "writer death must be visible to the UI while capture continues"
        );

        // Stopping a session whose writer already failed returns the error.
        assert!(controller.stop().is_err());
    }

    #[test]
    fn writer_failed_stays_false_through_a_clean_take() {
        let dir = tempfile::tempdir().unwrap();
        let config = RecordConfig {
            take_dir: dir.path().to_path_buf(),
            tracks: track_specs(1),
            channels: 1,
            sample_rate: 48_000,
        };
        let (mut sink, controller) = start_session(config).unwrap();
        sink.push_interleaved(&[0.25; 960]);
        thread::sleep(Duration::from_millis(60));
        // Healthy throughout — a clean take never flags failure.
        assert!(!controller.writer_failed());
        let counts = controller.stop().unwrap();
        assert_eq!(counts, vec![960]);
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
