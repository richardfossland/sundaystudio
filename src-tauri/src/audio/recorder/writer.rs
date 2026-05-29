//! Multi-track WAV writer — the disk side of the recording engine.
//!
//! One `hound` WAV file per track (`takes/{take}/{track}.wav`), 24-bit PCM at
//! the project sample rate. Runs on the writer thread, never the audio thread.
//!
//! Crash safety (a core promise — "a 90-minute recording must NEVER be lost"):
//! `flush` rewrites the RIFF/`data` chunk lengths incrementally, so a process
//! killed mid-recording leaves a *playable* WAV of everything written up to the
//! last flush. The writer thread flushes roughly every 250 ms.

use std::fs;
use std::io::BufWriter;
use std::path::Path;

use crate::error::{AppError, AppResult};

/// Broadcast-quality capture depth. 24-bit is the podcast/DAW norm: ample
/// headroom over 16-bit, half the size of 32-bit float.
pub const BITS_PER_SAMPLE: u16 = 24;
const I24_MAX: f32 = 8_388_607.0; // 2^23 - 1

/// What track a file belongs to and what to name it.
#[derive(Debug, Clone)]
pub struct TrackSpec {
    /// Stable track id (also used as the WAV filename stem).
    pub track_id: String,
}

impl TrackSpec {
    fn filename(&self) -> String {
        format!("{}.wav", self.track_id)
    }
}

/// Convert a normalised f32 sample (−1.0..1.0) to a 24-bit signed integer,
/// clamped so inter-sample overs can't wrap. hound writes the low 3 bytes.
fn f32_to_i24(x: f32) -> i32 {
    (x.clamp(-1.0, 1.0) * I24_MAX).round() as i32
}

/// Owns one `hound::WavWriter` per track plus a running sample count.
pub struct MultiTrackWriter {
    writers: Vec<hound::WavWriter<BufWriter<fs::File>>>,
    counts: Vec<u64>,
}

impl MultiTrackWriter {
    /// Create one WAV per track inside `dir` (created if missing).
    pub fn create(dir: &Path, tracks: &[TrackSpec], sample_rate: u32) -> AppResult<Self> {
        if tracks.is_empty() {
            return Err(AppError::Validation("no tracks to record".into()));
        }
        fs::create_dir_all(dir)?;
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate,
            bits_per_sample: BITS_PER_SAMPLE,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writers = Vec::with_capacity(tracks.len());
        for t in tracks {
            let path = dir.join(t.filename());
            let w = hound::WavWriter::create(&path, spec)
                .map_err(|e| AppError::Audio(format!("creating {}: {e}", path.display())))?;
            writers.push(w);
        }
        Ok(Self {
            counts: vec![0; tracks.len()],
            writers,
        })
    }

    /// Append a block of samples to one track.
    pub fn write_block(&mut self, track: usize, samples: &[f32]) -> AppResult<()> {
        let writer = self
            .writers
            .get_mut(track)
            .ok_or_else(|| AppError::Validation(format!("no such track index {track}")))?;
        for &s in samples {
            writer
                .write_sample(f32_to_i24(s))
                .map_err(|e| AppError::Audio(format!("writing sample: {e}")))?;
        }
        self.counts[track] += samples.len() as u64;
        Ok(())
    }

    /// Flush all tracks to disk, updating WAV headers so the files are playable
    /// even if the process dies before `finalize`.
    pub fn flush(&mut self) -> AppResult<()> {
        for w in &mut self.writers {
            w.flush()
                .map_err(|e| AppError::Audio(format!("flushing wav: {e}")))?;
        }
        Ok(())
    }

    /// Finalise every track (writes final headers) and return per-track sample
    /// counts.
    pub fn finalize(self) -> AppResult<Vec<u64>> {
        for w in self.writers {
            w.finalize()
                .map_err(|e| AppError::Audio(format!("finalising wav: {e}")))?;
        }
        Ok(self.counts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_per_track_files_with_correct_lengths() {
        let dir = tempfile::tempdir().unwrap();
        let tracks = vec![
            TrackSpec {
                track_id: "host".into(),
            },
            TrackSpec {
                track_id: "guest".into(),
            },
        ];
        let mut w = MultiTrackWriter::create(dir.path(), &tracks, 48_000).unwrap();

        w.write_block(0, &[0.5; 100]).unwrap();
        w.write_block(1, &[-0.25; 50]).unwrap();
        w.flush().unwrap();
        w.write_block(0, &[0.5; 100]).unwrap();
        let counts = w.finalize().unwrap();

        assert_eq!(counts, vec![200, 50]);

        // Reopen host track and verify it's a valid 24-bit WAV with our values.
        let host = hound::WavReader::open(dir.path().join("host.wav")).unwrap();
        assert_eq!(host.spec().bits_per_sample, 24);
        assert_eq!(host.spec().channels, 1);
        assert_eq!(host.len(), 200);
        let first: i32 = host.into_samples::<i32>().next().unwrap().unwrap();
        // 0.5 * (2^23 - 1) ≈ 4_194_303
        assert!((first - 4_194_303).abs() <= 1, "got {first}");
    }

    #[test]
    fn flush_leaves_a_playable_file_midway() {
        let dir = tempfile::tempdir().unwrap();
        let tracks = vec![TrackSpec {
            track_id: "t0".into(),
        }];
        let mut w = MultiTrackWriter::create(dir.path(), &tracks, 48_000).unwrap();
        w.write_block(0, &[0.1; 4800]).unwrap();
        w.flush().unwrap();

        // Without finalising, the on-disk file must already be readable.
        let r = hound::WavReader::open(dir.path().join("t0.wav")).unwrap();
        assert_eq!(r.len(), 4800);

        w.finalize().unwrap();
    }

    #[test]
    fn rejects_empty_track_set_and_bad_index() {
        let dir = tempfile::tempdir().unwrap();
        assert!(MultiTrackWriter::create(dir.path(), &[], 48_000).is_err());

        let mut w = MultiTrackWriter::create(
            dir.path(),
            &[TrackSpec {
                track_id: "t".into(),
            }],
            48_000,
        )
        .unwrap();
        assert!(w.write_block(9, &[0.0; 4]).is_err());
        w.finalize().unwrap();
    }

    #[test]
    fn clamps_out_of_range_samples() {
        assert_eq!(f32_to_i24(2.0), 8_388_607);
        assert_eq!(f32_to_i24(-2.0), -8_388_607);
        assert_eq!(f32_to_i24(0.0), 0);
    }
}
