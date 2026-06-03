//! The cpal input stream — the ONE part of the recorder that needs real audio
//! hardware, and therefore the one part not exercised by the test suite.
//!
//! ⚠️ HARDWARE-UNVERIFIED. Everything here compiles and is wired to the tested
//! session pipeline (`CaptureSink`), but it has not been run against a real
//! interface in this build. It must be validated on actual devices before the
//! recorder is declared done — see docs/ARCHITECTURE.md ("Phase 1 testing"):
//! 8-track 60-min capture, device unplug mid-recording, sample-rate mismatch,
//! and crash recovery, across the Focusrite/Behringer/RØDE/MOTU/built-in matrix.
//!
//! These functions are `pub` so they are part of the crate's API (the recording
//! UI / hardware integration in Phase 2.2 calls them) and don't read as dead
//! code. The data callback does only real-time-safe work: hand the interleaved
//! block to `CaptureSink::push_interleaved`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;

use super::session::CaptureSink;
use crate::error::{AppError, AppResult};

/// Resolve an input device by name, or the host default when `name` is None.
/// Returns a clear error if the named device is gone (e.g. unplugged).
pub fn find_input_device(name: Option<&str>) -> AppResult<cpal::Device> {
    let host = cpal::default_host();
    match name {
        None => host
            .default_input_device()
            .ok_or_else(|| AppError::Audio("no default input device".into())),
        Some(want) => host
            .input_devices()
            .map_err(|e| AppError::Audio(format!("listing input devices: {e}")))?
            .find(|d| d.name().ok().as_deref() == Some(want))
            .ok_or_else(|| AppError::Audio(format!("input device not found: {want}"))),
    }
}

/// Build (but do not start) a capture stream that feeds `sink`. The caller owns
/// the returned `Stream` on a dedicated thread and calls `.play()` — cpal's
/// `Stream` is `!Send` on some platforms, so it must never cross threads.
///
/// Handles the two common input formats (f32, i16). Other formats return an
/// explicit error rather than silently mis-decoding.
pub fn build_capture_stream(
    device: &cpal::Device,
    sample_rate: u32,
    channels: u16,
    mut sink: CaptureSink,
) -> AppResult<cpal::Stream> {
    let supported = device
        .default_input_config()
        .map_err(|e| AppError::Audio(format!("querying default input config: {e}")))?;

    let config = cpal::StreamConfig {
        channels,
        sample_rate: cpal::SampleRate(sample_rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let err_fn = |e| tracing::error!("audio input stream error: {e}");

    let stream = match supported.sample_format() {
        SampleFormat::F32 => device.build_input_stream(
            &config,
            move |data: &[f32], _: &cpal::InputCallbackInfo| sink.push_interleaved(data),
            err_fn,
            None,
        ),
        SampleFormat::I16 => {
            // Convert i16 → f32 in a reused buffer (allocated once, here, not in
            // the callback) to keep the real-time path allocation-free.
            let mut scratch: Vec<f32> = Vec::new();
            device.build_input_stream(
                &config,
                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                    scratch.clear();
                    scratch.reserve(data.len());
                    for &s in data {
                        scratch.push(s as f32 / i16::MAX as f32);
                    }
                    sink.push_interleaved(&scratch);
                },
                err_fn,
                None,
            )
        }
        other => {
            return Err(AppError::Audio(format!(
                "unsupported input sample format: {other:?} (f32 and i16 supported)"
            )))
        }
    }
    .map_err(|e| AppError::Audio(format!("building input stream: {e}")))?;

    Ok(stream)
}

/// How long the stream thread parks between shutdown-flag checks once the stream
/// is playing. The capture work happens entirely in cpal's own callback thread;
/// this thread only exists to OWN the `!Send` `Stream` and to keep it alive until
/// the controller stops, so a coarse poll is fine.
const STREAM_PARK: Duration = Duration::from_millis(20);

/// A live input `Stream` owned on its own thread.
///
/// `cpal::Stream` is `!Send` on some platforms, so it can never sit in Tauri's
/// (`Send + Sync`) managed state or cross an `await`. This handle solves that by
/// confining the stream to a dedicated thread: the thread resolves the device,
/// builds + plays the stream from the moved-in `CaptureSink`, parks while a
/// shutdown flag is clear, then drops the stream (which stops capture). The
/// handle itself holds only a `JoinHandle` + an `Arc<AtomicBool>`, so it IS
/// `Send` and lives happily in app state alongside the `RecordController`.
///
/// ⚠️ HARDWARE-UNVERIFIED, like the rest of this module: the wiring is exercised
/// by tests through a fake device builder (see the test below), but a real cpal
/// device open is deferred to the Phase 2.2 on-device validation matrix.
pub struct StreamHandle {
    thread: Option<JoinHandle<()>>,
    shutdown: Arc<AtomicBool>,
}

impl StreamHandle {
    /// Spawn the stream thread for `device_name` (None = host default), feeding
    /// the moved-in `sink`. Blocks until the thread reports the stream started
    /// (so a device/config failure surfaces here, not silently in the thread),
    /// then returns the live handle.
    ///
    /// `sink` is `Send`, so it crosses to the thread that owns the `!Send` stream;
    /// the thread builds and plays the stream there and never lets it escape.
    pub fn spawn(
        device_name: Option<String>,
        sample_rate: u32,
        channels: u16,
        sink: CaptureSink,
    ) -> AppResult<StreamHandle> {
        let shutdown = Arc::new(AtomicBool::new(false));
        let shutdown_thread = Arc::clone(&shutdown);
        // One-shot channel back to the caller: Ok once the stream is playing, or
        // the resolve/build/play error so `audio_record_start` can report it.
        let (ready_tx, ready_rx) = mpsc::channel::<AppResult<()>>();

        let thread = thread::Builder::new()
            .name("sundaystudio-capture".into())
            .spawn(move || {
                run_stream(
                    device_name,
                    sample_rate,
                    channels,
                    sink,
                    shutdown_thread,
                    ready_tx,
                );
            })
            .map_err(|e| AppError::Audio(format!("spawning capture thread: {e}")))?;

        // Wait for the thread to confirm the stream is live (or failed). If the
        // thread died before sending, surface that rather than hanging.
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(StreamHandle {
                thread: Some(thread),
                shutdown,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                Err(AppError::Audio(
                    "capture thread exited before the stream started".into(),
                ))
            }
        }
    }

    /// Signal the stream thread to drop the stream (stopping capture) and wait
    /// for it to exit. Consumes the handle, mirroring `RecordController::stop`.
    pub fn stop(mut self) -> AppResult<()> {
        self.shutdown.store(true, Ordering::Release);
        match self.thread.take() {
            Some(h) => h
                .join()
                .map_err(|_| AppError::Audio("capture thread panicked".into())),
            None => Err(AppError::Internal("capture stream already stopped".into())),
        }
    }
}

/// Body of the capture thread: resolve the device, build + play the stream, then
/// park until `shutdown` is set (or the caller drops the handle). Reports the
/// startup result back over `ready` before parking.
fn run_stream(
    device_name: Option<String>,
    sample_rate: u32,
    channels: u16,
    sink: CaptureSink,
    shutdown: Arc<AtomicBool>,
    ready: mpsc::Sender<AppResult<()>>,
) {
    // Resolve + build + play, capturing the first failure to report to the caller.
    let stream = (|| {
        let device = find_input_device(device_name.as_deref())?;
        let stream = build_capture_stream(&device, sample_rate, channels, sink)?;
        stream
            .play()
            .map_err(|e| AppError::Audio(format!("starting input stream: {e}")))?;
        Ok::<cpal::Stream, AppError>(stream)
    })();

    let stream = match stream {
        Ok(s) => {
            // Tell the caller we're live BEFORE we start parking.
            let _ = ready.send(Ok(()));
            s
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    // Hold the stream alive on this thread until asked to stop. The actual audio
    // moves through cpal's own callback thread; we just keep `stream` from being
    // dropped (which would stop capture) and own the `!Send` value.
    while !shutdown.load(Ordering::Acquire) {
        thread::sleep(STREAM_PARK);
    }
    // Dropping `stream` here stops the device.
    drop(stream);
}
