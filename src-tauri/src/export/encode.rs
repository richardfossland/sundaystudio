//! The ffmpeg encode step (Phase 7.1b) — turning the natively-written master WAV
//! into the lossy/archival delivery format (MP3 / AAC / FLAC).
//!
//! The renderer writes a pristine 24-bit master WAV via `hound`; only WAV is
//! native. Everything else is a deterministic re-encode that we hand to ffmpeg.
//! This module is split deliberately:
//!
//!   * [`EncodePlan`] + [`build_ffmpeg_args`] are **pure** — they validate a
//!     preset + sample-rate into a resolved plan and build the exact argument
//!     vector, with no process spawn. This mirrors the frontend `exportPlan.ts`
//!     so the two stay in lock-step, and it is the part we unit-test offline
//!     (no ffmpeg binary needed in the gate).
//!   * [`encode_with_ffmpeg`] is the **only** impure entry point: it spawns the
//!     bundled `ffmpeg` with the built args. When ffmpeg is absent it returns a
//!     typed [`EncodeError::FfmpegUnavailable`] so the caller can fall back to
//!     handing the user the master WAV rather than failing the whole export.
//!
//! NOTE: a real spawn is FFMPEG-SIDECAR-UNVERIFIED — the binary is bundled in a
//! later step. The arg *shape* and the missing-binary path are covered here; the
//! actual transcode is exercised on a rig with ffmpeg present.

use std::path::Path;
use std::process::Command;

use super::format::ExportFormat;

/// Sample rates we allow for export, in Hz. 48 kHz is the podcast/broadcast
/// default; 44.1 kHz is the legacy CD/MP3 rate some hosts still prefer. Mirrors
/// `ALLOWED_SAMPLE_RATES` in `exportPlan.ts`.
pub const ALLOWED_SAMPLE_RATES: [u32; 2] = [44_100, 48_000];

/// Bitrate bounds (kbps) for lossy encodes. Below 64 sounds bad for voice; above
/// 320 is wasteful and rejected by some hosts.
pub const MIN_BITRATE_KBPS: u32 = 64;
pub const MAX_BITRATE_KBPS: u32 = 320;

/// Whether a format encodes losslessly (no bitrate; ffmpeg uses a compression
/// knob instead). WAV is native; FLAC is a lossless ffmpeg encode.
pub fn is_lossless(format: ExportFormat) -> bool {
    matches!(format, ExportFormat::Wav | ExportFormat::Flac)
}

/// The ffmpeg audio codec for a format (the `-c:a` value). WAV is native and
/// never goes through ffmpeg, but we surface its PCM codec for completeness.
/// Mirrors `ffmpegCodec` in `exportPlan.ts`.
pub fn ffmpeg_codec(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Wav => "pcm_s24le",
        ExportFormat::Mp3 => "libmp3lame",
        ExportFormat::Aac => "aac",
        ExportFormat::Flac => "flac",
    }
}

/// A validation problem with an export request, caught before any process spawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EncodePlanError {
    /// Channels was not 1 (mono) or 2 (stereo).
    BadChannels(u16),
    /// Sample rate is not one of [`ALLOWED_SAMPLE_RATES`].
    BadSampleRate(u32),
    /// A bitrate was supplied for a lossless format (WAV/FLAC).
    BitrateOnLossless(u32),
    /// A lossy format (MP3/AAC) is missing its bitrate.
    MissingBitrate,
    /// Bitrate fell outside [`MIN_BITRATE_KBPS`]..=[`MAX_BITRATE_KBPS`].
    BitrateOutOfRange(u32),
}

impl std::fmt::Display for EncodePlanError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodePlanError::BadChannels(c) => {
                write!(f, "channels must be 1 (mono) or 2 (stereo), got {c}")
            }
            EncodePlanError::BadSampleRate(r) => {
                write!(f, "sample rate must be 44100 or 48000 Hz, got {r}")
            }
            EncodePlanError::BitrateOnLossless(b) => {
                write!(f, "lossless format takes no bitrate (got {b} kbps)")
            }
            EncodePlanError::MissingBitrate => write!(f, "lossy format needs a bitrate"),
            EncodePlanError::BitrateOutOfRange(b) => write!(
                f,
                "bitrate must be {MIN_BITRATE_KBPS}–{MAX_BITRATE_KBPS} kbps, got {b}"
            ),
        }
    }
}

/// A fully-resolved, validated encode plan: the effective parameters plus
/// whether the encode actually needs the ffmpeg sidecar (only WAV is native).
/// Mirrors the `ExportPlan` interface in `exportPlan.ts`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodePlan {
    pub format: ExportFormat,
    /// 1 = mono, 2 = stereo.
    pub channels: u16,
    pub sample_rate: u32,
    /// kbps for lossy formats; `None` for WAV/FLAC.
    pub bitrate_kbps: Option<u32>,
    /// ffmpeg `-c:a` codec name.
    pub codec: &'static str,
    /// True when the master WAV must be re-encoded by the sidecar.
    pub requires_encoder: bool,
    /// File extension (no dot), matching `ExportFormat::extension`.
    pub extension: &'static str,
}

/// Validate a format + bitrate + channels + sample-rate combination into an
/// [`EncodePlan`], or return the first problem found. Pure and total. Catches the
/// combinations that would make a downstream ffmpeg call meaningless — a bitrate
/// on a lossless format, a missing bitrate on a lossy one, an out-of-range
/// bitrate, a non-mono/stereo channel count, or an unsupported sample rate.
pub fn plan_encode(
    format: ExportFormat,
    bitrate_kbps: Option<u32>,
    channels: u16,
    sample_rate: u32,
) -> Result<EncodePlan, EncodePlanError> {
    if channels != 1 && channels != 2 {
        return Err(EncodePlanError::BadChannels(channels));
    }
    if !ALLOWED_SAMPLE_RATES.contains(&sample_rate) {
        return Err(EncodePlanError::BadSampleRate(sample_rate));
    }

    let lossless = is_lossless(format);
    let effective_bitrate = if lossless {
        if let Some(b) = bitrate_kbps {
            return Err(EncodePlanError::BitrateOnLossless(b));
        }
        None
    } else {
        let b = bitrate_kbps.ok_or(EncodePlanError::MissingBitrate)?;
        if !(MIN_BITRATE_KBPS..=MAX_BITRATE_KBPS).contains(&b) {
            return Err(EncodePlanError::BitrateOutOfRange(b));
        }
        Some(b)
    };

    Ok(EncodePlan {
        format,
        channels,
        sample_rate,
        bitrate_kbps: effective_bitrate,
        codec: ffmpeg_codec(format),
        requires_encoder: format != ExportFormat::Wav,
        extension: format.extension(),
    })
}

/// Build the ffmpeg argument vector that encodes the master WAV (`input`) into
/// the plan's format at `output`. Deterministic and order-stable so tests can
/// assert the exact vector. Mirrors `buildFfmpegArgs` in `exportPlan.ts`.
///
/// Shape: `-y -i <in> -c:a <codec> [-b:a <k>k] [-compression_level 8] -ar <rate>
/// -ac <ch> <out>`.
///   - `-y` overwrites without prompting (the renderer manages temp paths).
///   - `-b:a` is emitted only for lossy formats.
///   - FLAC adds `-compression_level 8` (best ratio; encode time is irrelevant
///     for an offline bounce).
///
/// Returns `None` for a WAV plan: WAV is written natively and never goes through
/// the sidecar, so asking for its args is a no-op rather than an error.
pub fn build_ffmpeg_args(plan: &EncodePlan, input: &Path, output: &Path) -> Option<Vec<String>> {
    if plan.format == ExportFormat::Wav {
        return None;
    }
    let mut args = vec![
        "-y".to_string(),
        "-i".to_string(),
        input.display().to_string(),
        "-c:a".to_string(),
        plan.codec.to_string(),
    ];
    if let Some(b) = plan.bitrate_kbps {
        args.push("-b:a".to_string());
        args.push(format!("{b}k"));
    }
    if plan.format == ExportFormat::Flac {
        args.push("-compression_level".to_string());
        args.push("8".to_string());
    }
    args.push("-ar".to_string());
    args.push(plan.sample_rate.to_string());
    args.push("-ac".to_string());
    args.push(plan.channels.to_string());
    args.push(output.display().to_string());
    Some(args)
}

/// What can go wrong when actually invoking the encoder.
#[derive(Debug)]
pub enum EncodeError {
    /// The ffmpeg binary could not be launched (not bundled / not on PATH). The
    /// caller can fall back to delivering the master WAV.
    FfmpegUnavailable(String),
    /// ffmpeg ran but exited non-zero; carries its captured stderr tail.
    FfmpegFailed { status: Option<i32>, stderr: String },
}

impl std::fmt::Display for EncodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EncodeError::FfmpegUnavailable(e) => write!(f, "ffmpeg unavailable: {e}"),
            EncodeError::FfmpegFailed { status, stderr } => {
                write!(f, "ffmpeg exited {status:?}: {stderr}")
            }
        }
    }
}

/// Encode the master WAV at `input` into `output` per `plan` by spawning ffmpeg.
///
/// This is the only function here that touches a process. It builds the args with
/// [`build_ffmpeg_args`] and runs the `ffmpeg` resolved by `ffmpeg_bin` (the
/// bundled sidecar path, or just `"ffmpeg"` to fall back to PATH). A WAV plan is
/// a programming error (the renderer writes WAV natively) and returns
/// `Ok(false)` — nothing was encoded. Otherwise `Ok(true)` on success.
///
/// FFMPEG-SIDECAR-UNVERIFIED: not exercised in the offline gate (no binary). The
/// argument shape and the missing-binary branch are unit-tested.
pub fn encode_with_ffmpeg(
    ffmpeg_bin: &str,
    plan: &EncodePlan,
    input: &Path,
    output: &Path,
) -> Result<bool, EncodeError> {
    let Some(args) = build_ffmpeg_args(plan, input, output) else {
        return Ok(false); // WAV — nothing to encode
    };
    let output_result = Command::new(ffmpeg_bin)
        .args(&args)
        .output()
        .map_err(|e| EncodeError::FfmpegUnavailable(e.to_string()))?;
    if output_result.status.success() {
        Ok(true)
    } else {
        // Keep only the tail of stderr — ffmpeg is chatty and we don't want to
        // surface megabytes into an error message.
        let stderr = String::from_utf8_lossy(&output_result.stderr);
        let tail: String = stderr.chars().rev().take(600).collect::<String>();
        let tail: String = tail.chars().rev().collect();
        Err(EncodeError::FfmpegFailed {
            status: output_result.status.code(),
            stderr: tail,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    #[test]
    fn lossless_classification_matches_frontend() {
        assert!(is_lossless(ExportFormat::Wav));
        assert!(is_lossless(ExportFormat::Flac));
        assert!(!is_lossless(ExportFormat::Mp3));
        assert!(!is_lossless(ExportFormat::Aac));
    }

    #[test]
    fn codecs_match_frontend() {
        assert_eq!(ffmpeg_codec(ExportFormat::Wav), "pcm_s24le");
        assert_eq!(ffmpeg_codec(ExportFormat::Mp3), "libmp3lame");
        assert_eq!(ffmpeg_codec(ExportFormat::Aac), "aac");
        assert_eq!(ffmpeg_codec(ExportFormat::Flac), "flac");
    }

    #[test]
    fn plans_a_valid_mp3() {
        let plan = plan_encode(ExportFormat::Mp3, Some(192), 2, 48_000).unwrap();
        assert_eq!(plan.bitrate_kbps, Some(192));
        assert_eq!(plan.codec, "libmp3lame");
        assert_eq!(plan.extension, "mp3");
        assert!(plan.requires_encoder);
    }

    #[test]
    fn plans_a_valid_flac_without_bitrate() {
        let plan = plan_encode(ExportFormat::Flac, None, 2, 44_100).unwrap();
        assert_eq!(plan.bitrate_kbps, None);
        assert!(plan.requires_encoder);
        assert_eq!(plan.extension, "flac");
    }

    #[test]
    fn wav_plan_needs_no_encoder() {
        let plan = plan_encode(ExportFormat::Wav, None, 2, 48_000).unwrap();
        assert!(!plan.requires_encoder);
        assert!(build_ffmpeg_args(&plan, &p("in.wav"), &p("out.wav")).is_none());
    }

    #[test]
    fn rejects_bad_channels() {
        assert_eq!(
            plan_encode(ExportFormat::Mp3, Some(128), 3, 48_000),
            Err(EncodePlanError::BadChannels(3))
        );
    }

    #[test]
    fn rejects_bad_sample_rate() {
        assert_eq!(
            plan_encode(ExportFormat::Mp3, Some(128), 2, 96_000),
            Err(EncodePlanError::BadSampleRate(96_000))
        );
    }

    #[test]
    fn rejects_bitrate_on_lossless() {
        assert_eq!(
            plan_encode(ExportFormat::Flac, Some(192), 2, 48_000),
            Err(EncodePlanError::BitrateOnLossless(192))
        );
        assert_eq!(
            plan_encode(ExportFormat::Wav, Some(192), 2, 48_000),
            Err(EncodePlanError::BitrateOnLossless(192))
        );
    }

    #[test]
    fn rejects_missing_and_out_of_range_bitrate() {
        assert_eq!(
            plan_encode(ExportFormat::Mp3, None, 2, 48_000),
            Err(EncodePlanError::MissingBitrate)
        );
        assert_eq!(
            plan_encode(ExportFormat::Mp3, Some(32), 2, 48_000),
            Err(EncodePlanError::BitrateOutOfRange(32))
        );
        assert_eq!(
            plan_encode(ExportFormat::Aac, Some(512), 2, 48_000),
            Err(EncodePlanError::BitrateOutOfRange(512))
        );
        // Boundaries are inclusive.
        assert!(plan_encode(ExportFormat::Mp3, Some(64), 1, 48_000).is_ok());
        assert!(plan_encode(ExportFormat::Mp3, Some(320), 2, 48_000).is_ok());
    }

    #[test]
    fn builds_lossy_args_in_expected_order() {
        let plan = plan_encode(ExportFormat::Mp3, Some(192), 2, 48_000).unwrap();
        let args = build_ffmpeg_args(&plan, &p("/tmp/master.wav"), &p("/tmp/out.mp3")).unwrap();
        assert_eq!(
            args,
            vec![
                "-y",
                "-i",
                "/tmp/master.wav",
                "-c:a",
                "libmp3lame",
                "-b:a",
                "192k",
                "-ar",
                "48000",
                "-ac",
                "2",
                "/tmp/out.mp3",
            ]
        );
    }

    #[test]
    fn builds_flac_args_with_compression_and_no_bitrate() {
        let plan = plan_encode(ExportFormat::Flac, None, 1, 44_100).unwrap();
        let args = build_ffmpeg_args(&plan, &p("/tmp/master.wav"), &p("/tmp/out.flac")).unwrap();
        assert_eq!(
            args,
            vec![
                "-y",
                "-i",
                "/tmp/master.wav",
                "-c:a",
                "flac",
                "-compression_level",
                "8",
                "-ar",
                "44100",
                "-ac",
                "1",
                "/tmp/out.flac",
            ]
        );
        // No bitrate flag on a lossless encode.
        assert!(!args.contains(&"-b:a".to_string()));
    }

    #[test]
    fn builds_aac_args() {
        let plan = plan_encode(ExportFormat::Aac, Some(160), 2, 48_000).unwrap();
        let args = build_ffmpeg_args(&plan, &p("in.wav"), &p("out.m4a")).unwrap();
        assert!(args.contains(&"aac".to_string()));
        assert!(args.contains(&"160k".to_string()));
        assert_eq!(args.last().unwrap(), "out.m4a");
    }

    #[test]
    fn missing_ffmpeg_is_a_typed_unavailable_error() {
        // A binary that cannot exist — the spawn must fail with FfmpegUnavailable,
        // never a panic, so the caller can fall back to the master WAV.
        let plan = plan_encode(ExportFormat::Mp3, Some(192), 2, 48_000).unwrap();
        let err = encode_with_ffmpeg(
            "ffmpeg-does-not-exist-sundaystudio",
            &plan,
            &p("in.wav"),
            &p("out.mp3"),
        )
        .unwrap_err();
        assert!(matches!(err, EncodeError::FfmpegUnavailable(_)));
    }

    #[test]
    fn encoding_a_wav_plan_is_a_noop() {
        let plan = plan_encode(ExportFormat::Wav, None, 2, 48_000).unwrap();
        // No process should be spawned for WAV; returns Ok(false).
        let did = encode_with_ffmpeg("ffmpeg", &plan, &p("in.wav"), &p("out.wav")).unwrap();
        assert!(!did);
    }
}
