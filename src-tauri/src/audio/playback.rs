//! The timeline playback engine (Phase 1.4).
//!
//! Recording and editing produce a timeline; the player needs to *hear* it back
//! and (later) monitor the master chain on real audio. This is the mirror image
//! of the recorder: where the recorder's cpal **input** callback de-interleaves
//! captured frames into rings that a writer thread drains to disk, playback runs
//! a render thread that fills a ring the cpal **output** callback drains to the
//! speakers.
//!
//! ```text
//!   timeline regions ──render thread──▶ interleaved ring ──drain──▶ cpal output
//!        (clips)              │                                          │
//!                            position atomic ◀── UI polls ───────────────┘
//! ```
//!
//! Split by testability, exactly like the recorder:
//! - `mix_playback_block` — the pure mixer: sum every track's placed clips at a
//!   timeline sample window, honouring per-track mute + gain, into one mono
//!   block. No state, no device; the unit tests drive it directly and assert the
//!   mix math (sum of regions, mute drops a track, gain scales it).
//! - `PlaybackState` — the lock-free control surface the render/output threads
//!   read each block: a `playing` flag, a `position` sample counter, and a
//!   per-track mute bitmask, all atomics so the UI→audio command queue can flip
//!   them without locking the real-time path.
//! - `start_playback` / `PlaybackController` — the ring + render thread + the
//!   UI-side handle; the FULL pipeline, driven in tests by pulling the output
//!   ring directly (no device).
//! - the cpal output stream lives in `recorder::stream`-style hardware code
//!   (Phase 2.2's transport); it is the only piece that needs a device.
//!
//! v1 plays the timeline dry: per-track gain + mute + the per-clip gain/fades the
//! editor already bakes into each region (see `export::render::render_region`).
//! No master DSP or effects on the playback bus yet — that is the v2 first pass,
//! the same way the monitor mixes dry (see `monitor.rs`).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rtrb::{Consumer, Producer, RingBuffer};

use crate::error::{AppError, AppResult};

/// The most timeline tracks we hold a mute bit for. Mirrors the monitor mixer's
/// ceiling — eight simultaneous mics is the product ceiling (see CLAUDE.md), so a
/// `u64` bitmask is ample and keeps the mute set lock-free on the audio thread.
pub const MAX_PLAYBACK_TRACKS: usize = 64;

/// Idle nap for the render thread when the output ring is full, so it doesn't
/// spin. Mirrors the writer thread's `IDLE_NAP`.
const IDLE_NAP: Duration = Duration::from_millis(5);

/// A single track ready to play: its assembled mono timeline (clips already
/// placed, trimmed, gained and faded at the sample level — see
/// `export::render::assemble_timeline`) plus the track's mixer gain. Mute is held
/// live in [`PlaybackState`] so the UI can toggle it during playback without
/// rebuilding the timeline.
#[derive(Debug, Clone)]
pub struct PlaybackTrack {
    /// Mono samples for the whole project timeline, index 0 = timeline start.
    pub timeline: Vec<f32>,
    /// Linear-to-dB mixer gain for the track (the `Track.gain_db` field).
    pub gain_db: f32,
}

/// Lock-free control surface for the player, shared between the UI (command
/// queue) and the render/output threads. Cheap to read every block.
#[derive(Debug)]
pub struct PlaybackState {
    /// Whether the transport is rolling. Paused = the render thread emits
    /// nothing and the position holds.
    playing: AtomicBool,
    /// Playhead position in timeline samples (mono frames from the start).
    position: AtomicU64,
    /// One bit per track: set = muted. Bit `i` ↔ track `i`.
    mute_mask: AtomicU64,
}

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            // Playback starts stopped: the UI presses play to roll, and we never
            // feed an output device until it does.
            playing: AtomicBool::new(false),
            position: AtomicU64::new(0),
            mute_mask: AtomicU64::new(0),
        }
    }
}

impl PlaybackState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Is the transport rolling?
    #[inline]
    pub fn playing(&self) -> bool {
        self.playing.load(Ordering::Acquire)
    }

    /// Start/stop rolling (UI side; flips the flag the render thread checks each
    /// block).
    pub fn set_playing(&self, on: bool) {
        self.playing.store(on, Ordering::Release);
    }

    /// Current playhead position in timeline samples.
    #[inline]
    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Acquire)
    }

    /// Move the playhead (a UI seek, or the render thread advancing it). The UI
    /// polls `position` to draw the playhead, so a seek is visible immediately.
    pub fn set_position(&self, samples: u64) {
        self.position.store(samples, Ordering::Release);
    }

    /// Advance the playhead by `samples` (the render thread does this per block),
    /// returning the new position.
    pub fn advance(&self, samples: u64) -> u64 {
        self.position.fetch_add(samples, Ordering::AcqRel) + samples
    }

    /// Is `track` muted? Out-of-range tracks read unmuted.
    #[inline]
    pub fn is_muted(&self, track: usize) -> bool {
        if track >= MAX_PLAYBACK_TRACKS {
            return false;
        }
        self.mute_mask.load(Ordering::Acquire) & (1u64 << track) != 0
    }

    /// Mute/unmute a track in the playback mix. A `fetch_and`/`fetch_or` keeps
    /// this a single lock-free RMW.
    pub fn set_muted(&self, track: usize, muted: bool) {
        if track >= MAX_PLAYBACK_TRACKS {
            return;
        }
        let bit = 1u64 << track;
        if muted {
            self.mute_mask.fetch_or(bit, Ordering::AcqRel);
        } else {
            self.mute_mask.fetch_and(!bit, Ordering::AcqRel);
        }
    }

    /// The raw mute bitmask (for tests / diagnostics).
    pub fn mute_mask(&self) -> u64 {
        self.mute_mask.load(Ordering::Acquire)
    }
}

/// Mix one block of the timeline into a mono output block, starting at the given
/// playhead sample. Sums every track's timeline sample for each frame in
/// `[start, start + frames)`, applying per-track linear gain and skipping muted
/// tracks. Real-time safe: writes into the caller-owned `out` (cleared and
/// resized first), no allocation, bounded work.
///
/// A track shorter than the window contributes silence past its end (it has
/// simply run out of audio), so tracks of different lengths mix cleanly — the
/// same padding discipline as the export mixer's [`mix_to_mono`].
///
/// [`mix_to_mono`]: crate::export::render::mix_to_mono
pub fn mix_playback_block(
    tracks: &[PlaybackTrack],
    state: &PlaybackState,
    start: u64,
    frames: usize,
    out: &mut Vec<f32>,
) {
    out.clear();
    out.resize(frames, 0.0);
    let start = start as usize;

    for (t, track) in tracks.iter().enumerate().take(MAX_PLAYBACK_TRACKS) {
        if state.is_muted(t) {
            continue;
        }
        let g = 10.0_f32.powf(track.gain_db / 20.0);
        for (f, slot) in out.iter_mut().enumerate() {
            let idx = start + f;
            if let Some(&s) = track.timeline.get(idx) {
                *slot += s * g;
            }
        }
    }
}

/// The longest track timeline among `tracks`, in samples. This is the playback
/// length: once the playhead passes it there is nothing left to hear.
pub fn timeline_len(tracks: &[PlaybackTrack]) -> u64 {
    tracks
        .iter()
        .map(|t| t.timeline.len() as u64)
        .max()
        .unwrap_or(0)
}

/// The UI-side handle to a running playback session.
pub struct PlaybackController {
    render_thread: Option<JoinHandle<()>>,
    state: Arc<PlaybackState>,
    /// Shutdown flag for the render thread (set on `stop`, mirrors the recorder).
    shutdown: Arc<AtomicBool>,
    /// Total timeline length in samples (so the UI can stop at the end / show a
    /// scrubber range without re-reading every track).
    length: u64,
    sample_rate: u32,
    /// Consumer of the rendered mono output. In production the cpal output
    /// callback owns this; tests drain it to assert the mix.
    output: Consumer<f32>,
}

impl PlaybackController {
    /// Is the transport rolling?
    pub fn playing(&self) -> bool {
        self.state.playing()
    }

    /// Start rolling from the current playhead.
    pub fn play(&self) {
        self.state.set_playing(true);
    }

    /// Pause: hold the playhead, stop feeding the output.
    pub fn pause(&self) {
        self.state.set_playing(false);
    }

    /// Current playhead in timeline samples (the UI polls this ~60fps).
    pub fn position(&self) -> u64 {
        self.state.position()
    }

    /// Current playhead in milliseconds (UI convenience).
    pub fn position_ms(&self) -> f64 {
        self.state.position() as f64 / self.sample_rate.max(1) as f64 * 1000.0
    }

    /// Seek the playhead to a sample position (clamped to the timeline length).
    pub fn seek(&self, samples: u64) {
        self.state.set_position(samples.min(self.length));
    }

    /// Seek the playhead to a millisecond position.
    pub fn seek_ms(&self, ms: f64) {
        let s = if ms <= 0.0 {
            0
        } else {
            (ms / 1000.0 * self.sample_rate as f64).round() as u64
        };
        self.seek(s);
    }

    /// Mute/unmute a track in the playback mix (the timeline is unaffected).
    pub fn set_mute(&self, track: usize, muted: bool) {
        self.state.set_muted(track, muted);
    }

    /// Is `track` muted in the playback mix?
    pub fn is_muted(&self, track: usize) -> bool {
        self.state.is_muted(track)
    }

    /// Total timeline length in samples.
    pub fn length(&self) -> u64 {
        self.length
    }

    /// Total timeline length in milliseconds (UI convenience).
    pub fn length_ms(&self) -> f64 {
        self.length as f64 / self.sample_rate.max(1) as f64 * 1000.0
    }

    /// Pop the next mono output sample, if any. The output callback drains this
    /// to the speakers; tests use it to assert the mix. Returns `None` when the
    /// ring is empty (paused / end of timeline / already drained).
    pub fn pop_output_sample(&mut self) -> Option<f32> {
        self.output.pop().ok()
    }

    /// Drain all currently-available output samples into `out` (cleared first),
    /// returning how many were read. A test/UI convenience over
    /// `pop_output_sample`.
    pub fn drain_output(&mut self, out: &mut Vec<f32>) -> usize {
        out.clear();
        while let Ok(s) = self.output.pop() {
            out.push(s);
        }
        out.len()
    }

    /// Stop playback: signal the render thread and wait for it to exit. The
    /// controller is consumed (mirrors `RecordController::stop`).
    pub fn stop(mut self) -> AppResult<()> {
        self.shutdown.store(true, Ordering::Release);
        self.state.set_playing(false);
        match self.render_thread.take() {
            Some(h) => h
                .join()
                .map_err(|_| AppError::Audio("playback render thread panicked".into())),
            None => Err(AppError::Internal("playback already stopped".into())),
        }
    }
}

/// How many frames the render thread mixes per pass. A small block keeps seek/
/// pause latency low while staying allocation-free (the scratch buffer is reused).
const RENDER_BLOCK: usize = 512;

/// Start a playback session: build the output ring, spawn the render thread, and
/// hand back the controller. Does NOT open any audio device — the cpal output
/// stream (Phase 2.2's transport) owns the `Consumer` end.
pub fn start_playback(
    tracks: Vec<PlaybackTrack>,
    sample_rate: u32,
) -> AppResult<PlaybackController> {
    if sample_rate == 0 {
        return Err(AppError::Validation("sample_rate must be > 0".into()));
    }
    if tracks.len() > MAX_PLAYBACK_TRACKS {
        return Err(AppError::Validation(format!(
            "tracks ({}) exceed the {MAX_PLAYBACK_TRACKS}-track playback ceiling",
            tracks.len()
        )));
    }

    let length = timeline_len(&tracks);
    let state = Arc::new(PlaybackState::new());
    let shutdown = Arc::new(AtomicBool::new(false));

    // The mono output ring, sized for ~1 second so a momentary output-callback
    // stall doesn't starve the speakers (mirrors the capture/monitor rings).
    let capacity = sample_rate.max(1) as usize;
    let (producer, consumer) = RingBuffer::<f32>::new(capacity);

    let render_thread = spawn_render(
        tracks,
        Arc::clone(&state),
        Arc::clone(&shutdown),
        length,
        producer,
    );

    Ok(PlaybackController {
        render_thread: Some(render_thread),
        state,
        shutdown,
        length,
        sample_rate,
        output: consumer,
    })
}

/// The render thread: while rolling, mix the timeline from the current playhead
/// into the output ring and advance the playhead; when paused or done, idle.
/// On shutdown, exit. Mirrors the recorder's `spawn_writer` discipline (a small
/// reused scratch buffer, an idle nap, a shutdown flag).
fn spawn_render(
    tracks: Vec<PlaybackTrack>,
    state: Arc<PlaybackState>,
    shutdown: Arc<AtomicBool>,
    length: u64,
    mut producer: Producer<f32>,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let mut scratch: Vec<f32> = Vec::with_capacity(RENDER_BLOCK);

        loop {
            if shutdown.load(Ordering::Acquire) {
                break;
            }

            // Paused, or the playhead reached the end of the timeline: stop
            // rolling and idle. (Reaching the end auto-pauses so the UI can show
            // a clean "stopped at end" without the playhead running off.)
            let pos = state.position();
            if !state.playing() || pos >= length {
                if pos >= length {
                    state.set_playing(false);
                }
                thread::sleep(IDLE_NAP);
                continue;
            }

            // Don't render past the end of the timeline this block.
            let remaining = (length - pos) as usize;
            let frames = RENDER_BLOCK.min(remaining);
            mix_playback_block(&tracks, &state, pos, frames, &mut scratch);

            // Push the block; if the ring is momentarily full the output callback
            // hasn't drained yet, so nap and retry the SAME block (don't advance
            // the playhead until the audio is actually queued).
            let mut pushed = 0usize;
            while pushed < scratch.len() {
                if shutdown.load(Ordering::Acquire) {
                    return;
                }
                match producer.push(scratch[pushed]) {
                    Ok(()) => pushed += 1,
                    Err(_) => thread::sleep(IDLE_NAP),
                }
            }
            state.advance(frames as u64);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(timeline: Vec<f32>, gain_db: f32) -> PlaybackTrack {
        PlaybackTrack { timeline, gain_db }
    }

    #[test]
    fn state_defaults_stopped_at_zero_unmuted() {
        let s = PlaybackState::new();
        assert!(!s.playing());
        assert_eq!(s.position(), 0);
        assert_eq!(s.mute_mask(), 0);
        assert!(!s.is_muted(0));
    }

    #[test]
    fn transport_flags_toggle_and_position_moves() {
        let s = PlaybackState::new();
        s.set_playing(true);
        assert!(s.playing());

        s.set_position(100);
        assert_eq!(s.position(), 100);
        assert_eq!(s.advance(50), 150);

        s.set_muted(3, true);
        assert!(s.is_muted(3));
        assert!(!s.is_muted(2));
        assert_eq!(s.mute_mask(), 0b1000);
        s.set_muted(3, false);
        assert_eq!(s.mute_mask(), 0);
    }

    #[test]
    fn out_of_range_track_is_ignored() {
        let s = PlaybackState::new();
        s.set_muted(MAX_PLAYBACK_TRACKS, true); // no-op, must not panic
        assert_eq!(s.mute_mask(), 0);
        assert!(!s.is_muted(MAX_PLAYBACK_TRACKS + 5));
    }

    #[test]
    fn mix_sums_all_tracks_per_frame() {
        // Two tracks, 4 frames each at 0.25 and 0.5 → mix 0.75 per frame.
        let tracks = [track(vec![0.25; 4], 0.0), track(vec![0.5; 4], 0.0)];
        let s = PlaybackState::new();
        let mut out = Vec::new();
        mix_playback_block(&tracks, &s, 0, 4, &mut out);
        assert_eq!(out.len(), 4);
        for v in out {
            assert!((v - 0.75).abs() < 1e-6, "got {v}");
        }
    }

    #[test]
    fn mute_drops_a_track_from_the_sum() {
        let tracks = [track(vec![0.25; 4], 0.0), track(vec![0.5; 4], 0.0)];
        let s = PlaybackState::new();
        s.set_muted(1, true); // drop the 0.5 track
        let mut out = Vec::new();
        mix_playback_block(&tracks, &s, 0, 4, &mut out);
        for v in out {
            assert!((v - 0.25).abs() < 1e-6, "got {v}");
        }
    }

    #[test]
    fn gain_scales_a_track() {
        // -6.0206 dB ≈ 0.5 linear: 1.0 sample plays back at ~0.5.
        let tracks = [track(vec![1.0; 4], -6.0206)];
        let s = PlaybackState::new();
        let mut out = Vec::new();
        mix_playback_block(&tracks, &s, 0, 4, &mut out);
        for v in out {
            assert!((v - 0.5).abs() < 1e-3, "got {v}");
        }
    }

    #[test]
    fn position_window_reads_from_the_right_offset() {
        // Ramp 0..8; play 4 frames starting at sample 2 → 2,3,4,5.
        let timeline: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let tracks = [track(timeline, 0.0)];
        let s = PlaybackState::new();
        let mut out = Vec::new();
        mix_playback_block(&tracks, &s, 2, 4, &mut out);
        assert_eq!(out, vec![2.0, 3.0, 4.0, 5.0]);
    }

    #[test]
    fn shorter_track_pads_with_silence_past_its_end() {
        // Track A is 6 long, B is 2 long; reading frames 0..6 sums B only where
        // it has audio, then A alone past sample 2.
        let a = track(vec![0.1; 6], 0.0);
        let b = track(vec![0.2; 2], 0.0);
        let s = PlaybackState::new();
        let mut out = Vec::new();
        mix_playback_block(&[a, b], &s, 0, 6, &mut out);
        assert!((out[0] - 0.3).abs() < 1e-6); // A + B
        assert!((out[1] - 0.3).abs() < 1e-6); // A + B
        assert!((out[2] - 0.1).abs() < 1e-6); // A only (B ran out)
        assert!((out[5] - 0.1).abs() < 1e-6);
    }

    #[test]
    fn reading_past_the_timeline_yields_silence() {
        let tracks = [track(vec![0.5; 4], 0.0)];
        let s = PlaybackState::new();
        let mut out = vec![9.0, 9.0]; // must be cleared
        mix_playback_block(&tracks, &s, 10, 3, &mut out);
        assert_eq!(out, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn timeline_len_is_the_longest_track() {
        let tracks = [track(vec![0.0; 100], 0.0), track(vec![0.0; 250], 0.0)];
        assert_eq!(timeline_len(&tracks), 250);
        assert_eq!(timeline_len(&[]), 0);
    }

    #[test]
    fn start_playback_rejects_zero_rate() {
        assert!(start_playback(vec![], 0).is_err());
    }

    #[test]
    fn start_playback_rejects_too_many_tracks() {
        let many = vec![track(vec![0.0; 1], 0.0); MAX_PLAYBACK_TRACKS + 1];
        assert!(start_playback(many, 48_000).is_err());
    }

    #[test]
    fn full_pipeline_plays_timeline_through_the_ring() {
        // Drive the WHOLE render-thread → ring path with NO audio device: we play
        // the role of the cpal output callback by draining the ring.
        let timeline: Vec<f32> = (0..2000).map(|i| (i % 7) as f32 * 0.01).collect();
        let expected = timeline.clone();
        let mut ctl = start_playback(vec![track(timeline, 0.0)], 48_000).unwrap();
        assert_eq!(ctl.length(), 2000);

        ctl.play();

        // Drain until the render thread reaches the end and auto-pauses, or we've
        // collected the whole timeline.
        let mut collected: Vec<f32> = Vec::new();
        let mut scratch = Vec::new();
        for _ in 0..200 {
            ctl.drain_output(&mut scratch);
            collected.extend_from_slice(&scratch);
            if collected.len() >= expected.len() && !ctl.playing() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }

        ctl.stop().unwrap();

        assert_eq!(collected.len(), expected.len(), "played the whole timeline");
        for (i, (a, b)) in collected.iter().zip(expected.iter()).enumerate() {
            assert!((a - b).abs() < 1e-6, "sample {i}: {a} vs {b}");
        }
    }

    #[test]
    fn paused_session_emits_nothing_and_holds_position() {
        let mut ctl = start_playback(vec![track(vec![0.5; 1000], 0.0)], 48_000).unwrap();
        // Never call play(): the render thread idles.
        thread::sleep(Duration::from_millis(20));
        let mut out = Vec::new();
        assert_eq!(ctl.drain_output(&mut out), 0, "no audio while paused");
        assert_eq!(ctl.position(), 0);
        ctl.stop().unwrap();
    }

    #[test]
    fn seek_clamps_to_the_timeline_length() {
        let ctl = start_playback(vec![track(vec![0.0; 480], 0.0)], 48_000).unwrap();
        ctl.seek(10_000); // way past the end
        assert_eq!(ctl.position(), 480);
        ctl.seek_ms(5.0); // 5ms @ 48k = 240 samples
        assert_eq!(ctl.position(), 240);
        assert!((ctl.position_ms() - 5.0).abs() < 0.05);
        ctl.stop().unwrap();
    }
}
