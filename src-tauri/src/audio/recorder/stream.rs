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

use cpal::traits::{DeviceTrait, HostTrait};
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
