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

/// Convert a millisecond position to a sample index at `rate` (rounded).
fn ms_to_samples(ms: f64, rate: u32) -> usize {
    if ms <= 0.0 {
        return 0;
    }
    (ms / 1000.0 * rate as f64).round() as usize
}

/// Render one region to audio: slice `[start_ms, end_ms)` out of the decoded
/// source take, apply the per-clip gain, then linear fade-in/out. Pure — the
/// caller supplies the source samples (mono, at `rate`). This is what makes
/// export region-aware: trim, gain and fades are baked per clip before mixing.
pub fn render_region(
    source: &[f32],
    rate: u32,
    start_ms: f64,
    end_ms: f64,
    fade_in_ms: f64,
    fade_out_ms: f64,
    gain_db: f32,
) -> Vec<f32> {
    let s = ms_to_samples(start_ms, rate).min(source.len());
    let e = ms_to_samples(end_ms, rate).min(source.len());
    if e <= s {
        return Vec::new();
    }
    let mut out = source[s..e].to_vec();

    let g = 10.0_f32.powf(gain_db / 20.0);
    if (g - 1.0).abs() > f32::EPSILON {
        for x in &mut out {
            *x *= g;
        }
    }

    let n = out.len();
    let fade_in = ms_to_samples(fade_in_ms, rate).min(n);
    for (i, x) in out.iter_mut().take(fade_in).enumerate() {
        *x *= i as f32 / fade_in as f32; // 0 → ~1 across the ramp
    }
    let fade_out = ms_to_samples(fade_out_ms, rate).min(n);
    for k in 0..fade_out {
        out[n - 1 - k] *= k as f32 / fade_out as f32; // 0 at the very end → up
    }
    out
}

/// A clip placed on a track timeline: its rendered mono audio at a ms position.
pub struct PlacedClip {
    pub position_ms: f64,
    pub samples: Vec<f32>,
}

/// Sum placed clips into one mono track-timeline buffer at `rate`. Overlapping
/// clips add, so two adjacent clips with matching fades crossfade naturally.
pub fn assemble_timeline(clips: &[PlacedClip], rate: u32) -> Vec<f32> {
    let len = clips
        .iter()
        .map(|c| ms_to_samples(c.position_ms, rate) + c.samples.len())
        .max()
        .unwrap_or(0);
    let mut buf = vec![0.0_f32; len];
    for c in clips {
        let off = ms_to_samples(c.position_ms, rate);
        for (i, &s) in c.samples.iter().enumerate() {
            if off + i < buf.len() {
                buf[off + i] += s;
            }
        }
    }
    buf
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
        .map(|l| loudness::reached_target(l, target.integrated_lufs))
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
        let a = MixSource {
            samples: vec![0.5; 4],
            gain_db: 0.0,
            mute: false,
        };
        let b = MixSource {
            samples: vec![0.5; 4],
            gain_db: 0.0,
            mute: false,
        };
        let muted = MixSource {
            samples: vec![1.0; 4],
            gain_db: 0.0,
            mute: true,
        };
        let mix = mix_to_mono(&[a, b, muted]);
        assert_eq!(mix, vec![1.0; 4]); // 0.5 + 0.5, muted ignored
    }

    #[test]
    fn mix_pads_to_longest_source() {
        let short = MixSource {
            samples: vec![0.2; 2],
            gain_db: 0.0,
            mute: false,
        };
        let long = MixSource {
            samples: vec![0.1; 5],
            gain_db: 0.0,
            mute: false,
        };
        assert_eq!(mix_to_mono(&[short, long]).len(), 5);
    }

    #[test]
    fn render_report_uses_canonical_reached_tolerance() {
        // Regression: render's embedded NormalizationReport.reached_target once used
        // a 1.0 LU tolerance while ExportResult.target_reached (and the documented
        // contract on NormalizationReport, plus normalize_clip_safe and the TS
        // preview) used 0.5 LU. An achieved loudness 0.7 LU off target then made the
        // two booleans about the *same* measurement disagree. Both must now use the
        // single canonical 0.5 LU verdict, so a 0.7-LU gap is NOT "reached".
        use crate::dsp::loudness::{reached_target, REACHED_TOLERANCE_LU};
        assert_eq!(REACHED_TOLERANCE_LU, 0.5);
        let target = -16.0_f32;
        // 0.7 LU off — inside the old 1.0 window but outside the canonical 0.5 one.
        assert!(
            !reached_target(target - 0.7, target),
            "0.7 LU off must not count as reached under the 0.5 LU contract"
        );
        // 0.4 LU off — inside the 0.5 window, reached.
        assert!(reached_target(target - 0.4, target));
        // Boundary is inclusive.
        assert!(reached_target(target - 0.5, target));
        assert!(!reached_target(target - 0.5001, target));
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
    fn render_region_slices_applies_gain_and_fades() {
        // 1s of DC=1.0 at SR; slice the middle 500ms with a 100ms fade each side
        // and −6 dB gain.
        let source = vec![1.0_f32; SR as usize];
        let out = render_region(&source, SR, 250.0, 750.0, 100.0, 100.0, -6.0);
        let half = SR as usize / 2; // 500ms
        assert_eq!(out.len(), half);
        let g = 10.0_f32.powf(-6.0 / 20.0);
        // Middle (past both fades) sits at the gain level.
        let mid = out[half / 2];
        assert!((mid - g).abs() < 1e-4, "mid {mid} vs {g}");
        // Edges are faded to (near) zero.
        assert!(out[0].abs() < 1e-4);
        assert!(out[half - 1].abs() < 1e-4);
        // Fade-in is monotonic up over its first 100ms.
        let fi = SR as usize / 10;
        assert!(out[fi / 2] < out[fi - 1]);
    }

    #[test]
    fn render_region_clamps_out_of_range_window() {
        let source = vec![0.5_f32; 100];
        // end beyond source length is clamped; start past end yields empty.
        assert!(render_region(&source, SR, 0.0, 10_000.0, 0.0, 0.0, 0.0).len() <= 100);
        assert!(render_region(&source, SR, 900.0, 100.0, 0.0, 0.0, 0.0).is_empty());
    }

    #[test]
    fn assemble_timeline_places_and_sums_overlap() {
        // Two 10-sample clips; the second starts where samples overlap → they add.
        let a = PlacedClip {
            position_ms: 0.0,
            samples: vec![1.0; 10],
        };
        let off_ms = 5.0 / SR as f64 * 1000.0; // 5 samples in
        let b = PlacedClip {
            position_ms: off_ms,
            samples: vec![1.0; 10],
        };
        let buf = assemble_timeline(&[a, b], SR);
        assert_eq!(buf.len(), 15); // 5 offset + 10
        assert_eq!(buf[0], 1.0); // only a
        assert_eq!(buf[5], 2.0); // a + b overlap
        assert_eq!(buf[14], 1.0); // only b tail
    }

    #[test]
    fn assemble_timeline_empty_is_empty() {
        assert!(assemble_timeline(&[], SR).is_empty());
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

    /// Write `samples` straight to disk as an integer-PCM WAV at `bits` bits and
    /// `channels` channels, bypassing our `write_wav` so we can exercise the
    /// decoder against externally-shaped files (8-bit, stereo, …). The buffer is
    /// interleaved; `samples` are clamped to [-1, 1] and quantised with hound's
    /// own per-bit-depth conversion.
    fn write_int_wav(path: &Path, samples: &[f32], channels: u16, bits: u16) {
        let spec = hound::WavSpec {
            channels,
            sample_rate: SR,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(path, spec).unwrap();
        let max = ((1i64 << (bits - 1)) - 1) as f32;
        for &s in samples {
            w.write_sample((s.clamp(-1.0, 1.0) * max).round() as i32)
                .unwrap();
        }
        w.finalize().unwrap();
    }

    #[test]
    fn reads_8bit_int_wav_without_dc_offset() {
        // hound stores 8-bit PCM unsigned (128 bias) but yields signed samples on
        // read, so `1/128` scaling is correct: silence stays at ~0, not +1.0.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("eightbit.wav");
        let signal: Vec<f32> = (0..256)
            .map(|i| 0.5 * ((i % 2) as f32 * 2.0 - 1.0))
            .collect();
        write_int_wav(&path, &signal, 1, 8);

        let (back, rate) = read_wav_mono(&path).unwrap();
        assert_eq!(rate, SR);
        assert_eq!(back.len(), signal.len());
        // No 128/255 DC offset leaked in: the mean sits at zero, not ~+1.0.
        let mean = back.iter().sum::<f32>() / back.len() as f32;
        assert!(
            mean.abs() < 0.02,
            "8-bit decode carried a DC offset: mean {mean}"
        );
        // 8-bit quantisation is coarse (1/128 ≈ 0.008 steps) but the shape holds.
        for (a, b) in signal.iter().zip(back.iter()) {
            assert!((a - b).abs() < 0.02, "{a} vs {b}");
        }
    }

    #[test]
    fn reads_24bit_int_wav_with_tiny_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("twentyfour.wav");
        let signal = sine(440.0, 0.5, 0.25);
        write_int_wav(&path, &signal, 1, 24);

        let (back, rate) = read_wav_mono(&path).unwrap();
        assert_eq!(rate, SR);
        assert_eq!(back.len(), signal.len());
        for (a, b) in signal.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-3, "{a} vs {b}");
        }
    }

    #[test]
    fn downmixes_stereo_to_mono_by_averaging() {
        // L = +A, R = -A → the average is silence; L = R = A → the average is A.
        let dir = tempfile::tempdir().unwrap();
        let cancel = dir.path().join("cancel.wav");
        let cancel_iv: Vec<f32> = [0.5_f32, -0.5].into_iter().cycle().take(64).collect();
        write_int_wav(&cancel, &cancel_iv, 2, 16);
        let (mono, _) = read_wav_mono(&cancel).unwrap();
        assert_eq!(mono.len(), 32); // 64 interleaved samples → 32 frames
        for s in &mono {
            assert!(s.abs() < 1e-2, "opposite channels should cancel: {s}");
        }

        let same = dir.path().join("same.wav");
        let same_iv: Vec<f32> = std::iter::repeat_n(0.5, 64).collect();
        write_int_wav(&same, &same_iv, 2, 16);
        let (mono, _) = read_wav_mono(&same).unwrap();
        assert_eq!(mono.len(), 32);
        for s in &mono {
            assert!(
                (s - 0.5).abs() < 1e-2,
                "identical channels should pass through: {s}"
            );
        }
    }

    #[test]
    fn reads_float_wav_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("float.wav");
        let signal = sine(330.0, 0.3, 0.1);
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: SR,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut w = hound::WavWriter::create(&path, spec).unwrap();
        for &s in &signal {
            w.write_sample(s).unwrap();
        }
        w.finalize().unwrap();

        let (back, rate) = read_wav_mono(&path).unwrap();
        assert_eq!(rate, SR);
        assert_eq!(back, signal); // float is lossless
    }
}
