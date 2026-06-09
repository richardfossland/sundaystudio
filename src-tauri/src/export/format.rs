//! Export formats and the platform-ready export presets (Phase 7.1).
//!
//! A preset bundles everything a hosting platform expects: the container/codec,
//! the bitrate, the channel layout, and which loudness target to normalise to
//! (see [`crate::dsp::loudness`]). One pick — "Spotify for Podcasters" — should
//! produce a file you can upload without thinking about kbps or LUFS.
//!
//! Only WAV is written natively (via `hound`). MP3/AAC/FLAC need the bundled
//! ffmpeg sidecar, which arrives in a later sub-phase — those presets carry
//! `requires_encoder = true` so the UI and the renderer know the master WAV is
//! the intermediate and the encode is the remaining step.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Output container/codec for an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ExportFormat.ts")]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    /// PCM WAV — pristine, written natively. Default for archival/re-editing.
    Wav,
    /// MP3 — the universal podcast format (needs the ffmpeg sidecar).
    Mp3,
    /// AAC — alternative lossy (needs the ffmpeg sidecar).
    Aac,
    /// FLAC — lossless archival (needs the ffmpeg sidecar).
    Flac,
}

impl ExportFormat {
    /// File extension (no dot).
    pub fn extension(self) -> &'static str {
        match self {
            ExportFormat::Wav => "wav",
            ExportFormat::Mp3 => "mp3",
            ExportFormat::Aac => "m4a",
            ExportFormat::Flac => "flac",
        }
    }

    /// Whether this format needs the (not-yet-bundled) ffmpeg encoder. Only WAV
    /// is written natively today.
    pub fn requires_encoder(self) -> bool {
        !matches!(self, ExportFormat::Wav)
    }
}

/// A platform-ready export preset, exposed to the UI picker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ExportPresetInfo.ts")]
pub struct ExportPresetInfo {
    pub id: String,
    pub label: String,
    pub format: ExportFormat,
    /// Encoder bitrate (kbps); `None` for lossless/PCM formats.
    pub bitrate_kbps: Option<u32>,
    /// 1 = mono, 2 = stereo.
    pub channels: u16,
    /// The loudness target id to normalise to (see `LoudnessTarget`).
    pub target_id: String,
    pub description: String,
    /// True when writing this format needs the ffmpeg sidecar (not WAV).
    pub requires_encoder: bool,
}

/// `const`-friendly preset definition (string literals, no `String`).
struct StaticPreset {
    id: &'static str,
    label: &'static str,
    format: ExportFormat,
    bitrate_kbps: Option<u32>,
    channels: u16,
    target_id: &'static str,
    description: &'static str,
}

impl StaticPreset {
    fn to_info(&self) -> ExportPresetInfo {
        ExportPresetInfo {
            id: self.id.to_string(),
            label: self.label.to_string(),
            format: self.format,
            bitrate_kbps: self.bitrate_kbps,
            channels: self.channels,
            target_id: self.target_id.to_string(),
            description: self.description.to_string(),
            requires_encoder: self.format.requires_encoder(),
        }
    }
}

const PRESETS: [StaticPreset; 5] = [
    StaticPreset {
        id: "spotify",
        label: "Spotify for Podcasters",
        format: ExportFormat::Mp3,
        bitrate_kbps: Some(192),
        channels: 2,
        target_id: "spotify",
        description: "192 kbps MP3, stereo, -14 LUFS. Ready to upload.",
    },
    StaticPreset {
        id: "apple-podcasts",
        label: "Apple Podcasts",
        format: ExportFormat::Mp3,
        bitrate_kbps: Some(192),
        channels: 2,
        target_id: "apple-podcasts",
        description: "192 kbps MP3, stereo, -16 LUFS.",
    },
    StaticPreset {
        id: "youtube",
        label: "YouTube (audio)",
        format: ExportFormat::Mp3,
        bitrate_kbps: Some(192),
        channels: 2,
        target_id: "youtube",
        description: "192 kbps MP3, stereo, -14 LUFS.",
    },
    StaticPreset {
        id: "general-podcast",
        label: "General podcast host",
        format: ExportFormat::Mp3,
        bitrate_kbps: Some(128),
        channels: 1,
        target_id: "apple-podcasts",
        description: "128 kbps mono, -16 LUFS — safe defaults for any host.",
    },
    StaticPreset {
        id: "wav-archival",
        label: "WAV (archival)",
        format: ExportFormat::Wav,
        bitrate_kbps: None,
        channels: 2,
        target_id: "apple-podcasts",
        description: "24-bit WAV, stereo, -16 LUFS. Pristine, for re-editing.",
    },
];

/// All export presets, for the format picker.
pub fn export_presets() -> Vec<ExportPresetInfo> {
    PRESETS.iter().map(StaticPreset::to_info).collect()
}

/// Look up an export preset by id.
pub fn preset_by_id(id: &str) -> Option<ExportPresetInfo> {
    PRESETS
        .iter()
        .find(|p| p.id == id)
        .map(StaticPreset::to_info)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_well_formed() {
        let presets = export_presets();
        assert_eq!(presets.len(), 5);
        assert!(preset_by_id("spotify").is_some());
        assert!(preset_by_id("nope").is_none());
        for p in presets {
            assert!(p.channels == 1 || p.channels == 2);
            assert!(crate::dsp::loudness::target_by_id(&p.target_id).is_some());
            // Lossy formats carry a bitrate; WAV does not.
            match p.format {
                ExportFormat::Wav | ExportFormat::Flac => {}
                _ => assert!(p.bitrate_kbps.is_some(), "{} missing bitrate", p.id),
            }
        }
    }

    #[test]
    fn only_wav_is_native() {
        assert!(!ExportFormat::Wav.requires_encoder());
        assert!(ExportFormat::Mp3.requires_encoder());
        assert_eq!(ExportFormat::Aac.extension(), "m4a");
        assert!(!preset_by_id("wav-archival").unwrap().requires_encoder);
    }
}
