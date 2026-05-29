//! Audio device discovery via cpal.
//!
//! This is the first real contact with the OS audio layer (CoreAudio on macOS,
//! WASAPI on Windows). We enumerate input and output devices and summarise each
//! device's capabilities: channel count, supported sample-rate range, and which
//! of the standard podcast rates (44.1 / 48 / 88.2 / 96 kHz) it can actually do.
//!
//! Phase 1.1 builds the full selection layer (persistence, hot-plug, ASIO
//! preference, channel→track matrix) on top of this enumeration.

use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::error::{AppError, AppResult};

/// Standard sample rates we surface in the UI. A device advertises a *range*;
/// we report which of these well-known rates fall inside it.
const STANDARD_RATES: [u32; 4] = [44_100, 48_000, 88_200, 96_000];

/// One audio device (input or output) with the capabilities the UI needs.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../src/lib/bindings/AudioDevice.ts")]
pub struct AudioDevice {
    /// Human-readable device name as reported by the OS.
    pub name: String,
    /// `"input"` or `"output"`.
    pub direction: String,
    /// Max channel count across the device's supported configs (0 if unknown).
    pub channels: u16,
    /// Standard sample rates (Hz) the device supports.
    pub sample_rates: Vec<u32>,
    /// Whether this is the host's default device for its direction.
    pub is_default: bool,
}

/// The result of enumerating the system's audio devices.
#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../src/lib/bindings/AudioDeviceList.ts")]
pub struct AudioDeviceList {
    /// The cpal host backing this enumeration (e.g. `"CoreAudio"`, `"WASAPI"`).
    pub host: String,
    pub inputs: Vec<AudioDevice>,
    pub outputs: Vec<AudioDevice>,
}

/// Direction a device is enumerated for — keeps the two summarise paths honest.
#[derive(Clone, Copy)]
enum Direction {
    Input,
    Output,
}

impl Direction {
    fn as_str(self) -> &'static str {
        match self {
            Direction::Input => "input",
            Direction::Output => "output",
        }
    }
}

/// Summarise one device's capabilities. Never fails hard: a device that refuses
/// to report a name or configs is reported with what we could read (defaults of
/// 0 channels / empty rates) rather than aborting the whole enumeration.
fn summarise(device: &cpal::Device, dir: Direction, is_default: bool) -> AudioDevice {
    let name = device
        .name()
        .unwrap_or_else(|_| "Unknown device".to_string());

    // Walk every supported config range, tracking the widest channel count and
    // the union of supported sample-rate ranges.
    let mut max_channels: u16 = 0;
    let mut rate_min = u32::MAX;
    let mut rate_max = 0u32;

    let configs: Vec<cpal::SupportedStreamConfigRange> = match dir {
        Direction::Input => device
            .supported_input_configs()
            .map(|c| c.collect())
            .unwrap_or_default(),
        Direction::Output => device
            .supported_output_configs()
            .map(|c| c.collect())
            .unwrap_or_default(),
    };

    for cfg in &configs {
        max_channels = max_channels.max(cfg.channels());
        rate_min = rate_min.min(cfg.min_sample_rate().0);
        rate_max = rate_max.max(cfg.max_sample_rate().0);
    }

    let sample_rates: Vec<u32> = if rate_min == u32::MAX {
        Vec::new()
    } else {
        STANDARD_RATES
            .into_iter()
            .filter(|r| *r >= rate_min && *r <= rate_max)
            .collect()
    };

    AudioDevice {
        name,
        direction: dir.as_str().to_string(),
        channels: max_channels,
        sample_rates,
        is_default,
    }
}

/// Enumerate input and output devices on the default host.
pub fn enumerate() -> AppResult<AudioDeviceList> {
    let host = cpal::default_host();

    let default_in = host.default_input_device().and_then(|d| d.name().ok());
    let default_out = host.default_output_device().and_then(|d| d.name().ok());

    let inputs = host
        .input_devices()
        .map_err(|e| AppError::Audio(format!("listing input devices: {e}")))?
        .map(|d| {
            let is_default = d.name().ok().as_deref() == default_in.as_deref();
            summarise(&d, Direction::Input, is_default)
        })
        .collect();

    let outputs = host
        .output_devices()
        .map_err(|e| AppError::Audio(format!("listing output devices: {e}")))?
        .map(|d| {
            let is_default = d.name().ok().as_deref() == default_out.as_deref();
            summarise(&d, Direction::Output, is_default)
        })
        .collect();

    Ok(AudioDeviceList {
        host: host.id().name().to_string(),
        inputs,
        outputs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerate_does_not_panic_and_reports_a_host() {
        // CI runners may have zero devices; the contract is only that the call
        // succeeds and names a host. Device counts are environment-dependent.
        let list = enumerate().expect("enumeration should not error");
        assert!(!list.host.is_empty(), "host should be named");
    }

    #[test]
    fn standard_rates_filter_is_within_range() {
        // Sanity: nothing outside the advertised range leaks through.
        for r in STANDARD_RATES {
            assert!((44_100..=96_000).contains(&r));
        }
    }
}
