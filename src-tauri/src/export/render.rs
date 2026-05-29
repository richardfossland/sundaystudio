//! The export renderer: mix → master → loudness-normalise → interleaved PCM,
//! plus the WAV read/write helpers (Phase 7.1).
//!
//! This is the bounce. It sums the project's per-track mono WAVs into a single
//! bus, runs the chosen [`MasterPreset`](crate::dsp::master::MasterPreset) chain,
//! and normalises to the platform's integrated-loudness target — the same DSP as
//! Phase 4.2, applied to the whole show at once. The result is an interleaved
//! buffer ready to write as WAV (and, later, to hand to the ffmpeg sidecar).
//!
//! Mastering is mono (the effects are mono and a church podcast is voice-first):
//! we mix and process on a mono bus, then expand to the output channel layout
//! (mono, or dual-mono stereo). Crucially, loudness is measured and normalised on
//! the *expanded* buffer — dual-mono stereo measures ~3 LU louder than the same
//! mono signal (R128 sums channel energies), so normalising the mono bus and then
//! duplicating would miss the target by 3 LU. True per-channel stereo mastering
//! is a later refinement; for now stereo output is faithful dual-mono.

use std::path::Path;

use crate::dsp::loudness::{self, LoudnessError, LoudnessTarget, NormalizationReport};
use crate::dsp::master::MasterPreset;
use crate::dsp::Effect;

/// A mono source track to fold into the mix, with its mixer state.
pub struct MixSource {
    pub samples: Vec<f32>,
    pub gain_db: f32,
    pub mute: bool,
}

/// Sum non-muted, gain-applied mono sources, padding shorter ones with silence.
/// Returns an empty buffer if there's nothing audible to mix.
pub fn mix_to_mono(sources: &[MixSource]) -> Vec<f32> {
    let len = sources
        .iter()
        .filter(|s| !s.mute)
        .map(|s| s.samples.len())
        .max()
        .unwrap_or(0);
    let mut mix = vec![0.0_f32; len];
    for src in sources.iter().filter(|s| !s.mute) {
        let g = 10.0_f32.powf(src.gain_db / 20.0);
        for (m, &s) in mix.iter_mut().zip(src.samples.iter()) {
            *m += s * g;
        }
    }
    mix
}

/// Expand a mono buffer to an interleaved buffer of `channels` (dual-mono).
fn expand(mono: &[f32], channels: u16) -> Vec<f32> {
    if channels <= 1 {
        return mono.to_vec();
    }
    let mut out = Vec::with_capacity(mono.len() * channels as usize);
    for &s in mono {
        for _ in 0..channels {
            out.push(s);
        }
    }
    out
}

/// Run the limiter independently over each channel of an interleaved buffer.
/// (For dual-mono the channels are identical, so the results match; doing it
/// per-channel keeps it correct for any layout.)
fn limit_interleaved(out: &mut [f32], channels: u16, limiter: &mut crate::dsp::limiter::Limiter) {
    let ch = channels.max(1) as usize;
    if ch == 1 {
        limiter.process(out);
        return;
    }
    let frames = out.len() / ch;
    let mut chan = vec![0.0_f32; frames];
    for c in 0..ch {
        for (i, slot) in chan.iter_mut().enumerate() {
            *slot = out[i * ch + c];
        }
        limiter.reset();
        limiter.process(&mut chan);
        for (i, &v) in chan.iter().enumerate() {
            out[i * ch + c] = v;
        }
    }
}

/// Render the mix to an interleaved buffer normalised to `target`.
///
/// Pipeline: mix → EQ + multiband glue (mono) → expand to `channels` → loudness
/// gain measured on the expanded buffer → per-channel limiter (the ceiling). The
/// report's `before` is the raw mix and `after` is the final output, both in the
/// output channel layout so the numbers are comparable.
pub fn render(
    sources: &[MixSource],
    channels: u16,
    rate: u32,
    master: MasterPreset,
    target: &LoudnessTarget,
) -> Result<(Vec<f32>, NormalizationReport), LoudnessError> {
    let channels = channels.max(1);
    let mono = mix_to_mono(sources);

    // Raw-mix loudness in the output layout, for an honest before/after report.
    let before = loudness::measure(&expand(&mono, channels), channels as u32, rate)?;

    let mut chain = master.build();
    chain.prepare(rate as f32);

    // Tone + glue on the mono bus.
    let mut bus = mono;
    chain.eq.process(&mut bus);
    chain.multiband.process(&mut bus);

    // Expand to the output layout, then compute the loudness gain against it.
    let mut out = expand(&bus, channels);
    let glued = loudness::measure(&out, channels as u32, rate)?;
    let gain_db = glued
        .integrated_lufs
        .map(|l| target.integrated_lufs - l)
        .unwrap_or(0.0);
    let lin = 10.0_f32.powf(gain_db / 20.0);
    for s in out.iter_mut() {
        *s *= lin;
    }

    // Limiter last guarantees the true-peak ceiling.
    chain.limiter.ceiling_db = target.true_peak_ceiling_dbtp;
    chain.limiter.prepare(rate as f32);
    limit_interleaved(&mut out, channels, &mut chain.limiter);

    let after = loudness::measure(&out, channels as u32, rate)?;
    let reached_target = after
        .integrated_lufs
        .map(|l| (l - target.integrated_lufs).abs() <= 1.0)
        .unwrap_or(false);

    Ok((
        out,
        NormalizationReport {
            target_lufs: target.integrated_lufs,
            gain_applied_db: gain_db,
            before,
            after,
            gain_capped_by_peak: false,
            reached_target,
        },
    ))
}

/// Read a WAV into mono f32 in [-1, 1] (downmixing by averaging if multichannel),
/// returning the samples and the file's sample rate.
pub fn read_wav_mono(path: &Path) -> Result<(Vec<f32>, u32), String> {
    let mut reader = hound::WavReader::open(path).map_err(|e| format!("open: {e}"))?;
    let spec = reader.spec();
    let ch = spec.channels.max(1) as usize;

    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader
            .samples::<f32>()
            .collect::<Result<_, _>>()
            .map_err(|e| format!("decode: {e}"))?,
        hound::SampleFormat::Int => {
            let scale = 1.0_f32 / (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<_, _>>()
                .map_err(|e| format!("decode: {e}"))?
        }
    };

    let mono = if ch == 1 {
        interleaved
    } else {
        interleaved
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    };
    Ok((mono, spec.sample_rate))
}

/// Write an interleaved f32 buffer to a PCM WAV (`bits` = 16 or 24). Returns the
/// number of bytes written.
pub fn write_wav(
    path: &Path,
    interleaved: &[f32],
    channels: u16,
    rate: u32,
    bits: u16,
) -> Result<u64, String> {
    let spec = hound::WavSpec {
        channels: channels.max(1),
        sample_rate: rate,
        bits_per_sample: bits,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).map_err(|e| format!("create: {e}"))?;
    let max = ((1i64 << (bits - 1)) - 1) as f32;
    for &s in interleaved {
        let v = (s.clamp(-1.0, 1.0) * max).round() as i32;
        writer.write_sample(v).map_err(|e| format!("write: {e}"))?;
    }
    writer.finalize().map_err(|e| format!("finalize: {e}"))?;
    std::fs::metadata(path)
        .map(|m| m.len())
        .map_err(|e| format!("stat: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::loudness::target_by_id;
    use std::f32::consts::TAU;

    const SR: u32 = 48_000;

    fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
        let n = (SR as f32 * secs) as usize;
        (0..n)
            .map(|i| amp * (freq * TAU * i as f32 / SR as f32).sin())
            .collect()
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0_f32, |m, &s| m.max(s.abs()))
    }

    #[test]
    fn mix_sums_unmuted_and_applies_gain() {
        let a = MixSource { samples: vec![0.5; 4], gain_db: 0.0, mute: false };
        let b = MixSource { samples: vec![0.5; 4], gain_db: 0.0, mute: false };
        let muted = MixSource { samples: vec![1.0; 4], gain_db: 0.0, mute: true };
        let mix = mix_to_mono(&[a, b, muted]);
        assert_eq!(mix, vec![1.0; 4]); // 0.5 + 0.5, muted ignored
    }

    #[test]
    fn mix_pads_to_longest_source() {
        let short = MixSource { samples: vec![0.2; 2], gain_db: 0.0, mute: false };
        let long = MixSource { samples: vec![0.1; 5], gain_db: 0.0, mute: false };
        assert_eq!(mix_to_mono(&[short, long]).len(), 5);
    }

    #[test]
    fn render_mono_reaches_target_without_clipping() {
        let sources = vec![MixSource {
            samples: sine(300.0, 0.15, 6.0),
            gain_db: 0.0,
            mute: false,
        }];
        let target = target_by_id("apple-podcasts").unwrap(); // -16 LUFS
        let (out, report) = render(&sources, 1, SR, MasterPreset::Sermon, &target).unwrap();
        assert!(peak(&out) <= 1.0, "clipped: {}", peak(&out));
        let achieved = report.after.integrated_lufs.unwrap();
        assert!(
            (achieved - target.integrated_lufs).abs() <= 1.5,
            "achieved {achieved}, target {}",
            target.integrated_lufs
        );
    }

    #[test]
    fn render_stereo_hits_target_in_its_own_layout() {
        // Dual-mono stereo must still land on the LUFS target (the +3 LU channel-
        // summing trap): we measure/normalise on the expanded buffer.
        let sources = vec![MixSource {
            samples: sine(220.0, 0.2, 6.0),
            gain_db: 0.0,
            mute: false,
        }];
        let target = target_by_id("spotify").unwrap(); // -14 LUFS
        let (out, report) =
            render(&sources, 2, SR, MasterPreset::ConversationPodcast, &target).unwrap();
        assert_eq!(out.len(), report_frames(&out, 2) * 2);
        let achieved = report.after.integrated_lufs.unwrap();
        assert!(
            (achieved - target.integrated_lufs).abs() <= 1.5,
            "stereo achieved {achieved}, target {}",
            target.integrated_lufs
        );
        assert!(peak(&out) <= 1.0);
    }

    fn report_frames(interleaved: &[f32], ch: usize) -> usize {
        interleaved.len() / ch
    }

    #[test]
    fn empty_mix_renders_to_silence() {
        let target = target_by_id("spotify").unwrap();
        let (out, report) = render(&[], 1, SR, MasterPreset::MusicHeavy, &target).unwrap();
        assert!(out.is_empty());
        assert_eq!(report.before.integrated_lufs, None);
    }

    #[test]
    fn wav_round_trip_preserves_signal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.wav");
        let signal = sine(440.0, 0.5, 0.5);
        let bytes = write_wav(&path, &signal, 1, SR, 24).unwrap();
        assert!(bytes > 0);
        let (back, rate) = read_wav_mono(&path).unwrap();
        assert_eq!(rate, SR);
        assert_eq!(back.len(), signal.len());
        // 24-bit quantisation error is tiny.
        for (a, b) in signal.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }
}
