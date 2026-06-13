//! Streaming sample-rate conversion for the capture path.
//!
//! When the input device cannot open at the project rate (see [`super::rate`]),
//! the device is opened at a supported rate and every captured block is
//! resampled to the project rate here, BEFORE it reaches the `CaptureSink`, so
//! the WAVs on disk are always at the project rate (no wrong-speed playback).
//!
//! The cpal callback hands us interleaved blocks of *arbitrary* length, but
//! `rubato`'s fixed resamplers consume a fixed number of input frames per call.
//! [`BlockResampler`] bridges that: it accumulates de-interleaved samples in
//! per-channel queues and drains them in `input_frames_next()`-sized chunks,
//! re-interleaving the resampled output. Buffers are allocated once at
//! construction and reused, so steady-state conversion does not allocate.
//!
//! ⚠️ This wiring is exercised by unit tests with synthetic blocks, but the
//! full live capture → resample → disk path needs a real device that runs at a
//! non-project rate (e.g. a 44.1 kHz-locked interface in a 48 kHz project) to
//! verify end to end — see `stream.rs`'s hardware-unverified banner.

use rubato::{FastFixedIn, PolynomialDegree, Resampler};

use crate::error::{AppError, AppResult};

/// Resamples interleaved capture blocks from `input_rate` to `output_rate`.
pub struct BlockResampler {
    inner: FastFixedIn<f32>,
    channels: usize,
    /// Frames the resampler wants per `process` call.
    chunk_frames: usize,
    /// Per-channel pending (de-interleaved) input not yet fed to the resampler.
    pending: Vec<Vec<f32>>,
    /// Per-channel fixed-size input scratch handed to `process` (len == chunk).
    in_buf: Vec<Vec<f32>>,
    /// Per-channel output scratch the resampler writes into.
    out_buf: Vec<Vec<f32>>,
    /// Reused interleaved output handed back to the caller.
    interleaved_out: Vec<f32>,
}

impl BlockResampler {
    /// Build a resampler from `input_rate` to `output_rate` for `channels`.
    /// `chunk_frames` is the resampler's fixed input block size; a value around
    /// a typical callback size (e.g. 1024) keeps latency and allocation low.
    pub fn new(
        input_rate: u32,
        output_rate: u32,
        channels: usize,
        chunk_frames: usize,
    ) -> AppResult<Self> {
        if channels == 0 {
            return Err(AppError::Validation(
                "resampler channels must be > 0".into(),
            ));
        }
        if input_rate == 0 || output_rate == 0 {
            return Err(AppError::Validation("resampler rates must be > 0".into()));
        }
        let ratio = output_rate as f64 / input_rate as f64;
        let inner = FastFixedIn::<f32>::new(
            ratio,
            // We never re-tune the ratio at runtime, so the relative bound can be
            // tight (1.0 allows no further change). A small margin avoids edge
            // rounding issues without enabling runtime ratio changes we don't use.
            1.1,
            PolynomialDegree::Cubic,
            chunk_frames,
            channels,
        )
        .map_err(|e| AppError::Audio(format!("creating resampler: {e}")))?;

        let in_buf = vec![vec![0.0f32; chunk_frames]; channels];
        let out_buf = inner.output_buffer_allocate(true);

        Ok(BlockResampler {
            inner,
            channels,
            chunk_frames,
            pending: vec![Vec::new(); channels],
            in_buf,
            out_buf,
            interleaved_out: Vec::new(),
        })
    }

    /// Feed one interleaved block (`len == frames * channels`) and return the
    /// resampled interleaved output produced so far. Output is empty until at
    /// least one full input chunk has accumulated; thereafter it tracks the
    /// resample ratio. The returned slice is owned by `self` and is overwritten
    /// on the next call.
    pub fn process_interleaved(&mut self, input: &[f32]) -> AppResult<&[f32]> {
        let ch = self.channels;
        self.interleaved_out.clear();

        // De-interleave into the pending per-channel queues.
        if !input.is_empty() {
            let frames = input.len() / ch;
            for c in 0..ch {
                let q = &mut self.pending[c];
                let mut i = c;
                for _ in 0..frames {
                    q.push(input[i]);
                    i += ch;
                }
            }
        }

        // Drain whole chunks while we have at least `chunk_frames` queued.
        while self.pending[0].len() >= self.chunk_frames {
            for c in 0..ch {
                // Copy the next chunk out of the pending queue into the fixed
                // input scratch, then remove it from the queue.
                self.in_buf[c].clear();
                self.in_buf[c].extend(self.pending[c].drain(..self.chunk_frames));
            }

            let (_in_used, out_frames) = self
                .inner
                .process_into_buffer(&self.in_buf, &mut self.out_buf, None)
                .map_err(|e| AppError::Audio(format!("resampling block: {e}")))?;

            // Re-interleave the produced output frames.
            let base = self.interleaved_out.len();
            self.interleaved_out.resize(base + out_frames * ch, 0.0);
            for c in 0..ch {
                let src = &self.out_buf[c];
                let mut idx = base + c;
                for &s in src.iter().take(out_frames) {
                    self.interleaved_out[idx] = s;
                    idx += ch;
                }
            }
        }

        Ok(&self.interleaved_out)
    }

    /// The resampler's fixed input chunk size (frames), exposed for tests.
    #[cfg(test)]
    pub fn chunk_frames(&self) -> usize {
        self.chunk_frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_zero_channels_and_rates() {
        assert!(BlockResampler::new(48_000, 48_000, 0, 1024).is_err());
        assert!(BlockResampler::new(0, 48_000, 1, 1024).is_err());
        assert!(BlockResampler::new(48_000, 0, 1, 1024).is_err());
    }

    #[test]
    fn no_output_before_a_full_chunk_accumulates() {
        let mut r = BlockResampler::new(44_100, 48_000, 1, 1024).unwrap();
        // Feed fewer than `chunk_frames` mono samples → nothing comes out yet.
        let half = vec![0.1f32; 512];
        let out = r.process_interleaved(&half).unwrap();
        assert!(out.is_empty(), "no full chunk yet, expected empty output");
    }

    #[test]
    fn upsampling_produces_more_frames_than_it_consumes() {
        // 44.1 → 48 kHz: output frames ≈ input * 48000/44100 ≈ input * 1.088.
        let chunk = 1024usize;
        let mut r = BlockResampler::new(44_100, 48_000, 1, chunk).unwrap();
        // Feed several full chunks of mono audio so the steady-state ratio shows.
        let total_in = chunk * 8;
        let block = vec![0.25f32; total_in];
        let out_frames = r.process_interleaved(&block).unwrap().len(); // mono → frames == samples
        let expected = (total_in as f64 * 48_000.0 / 44_100.0).round() as usize;
        // Allow for the resampler's small startup/edge transient.
        let diff = (out_frames as i64 - expected as i64).unsigned_abs();
        assert!(
            diff <= chunk as u64,
            "got {out_frames} frames, expected ~{expected} (diff {diff})"
        );
        assert!(out_frames > total_in, "upsampling must yield more frames");
    }

    #[test]
    fn downsampling_produces_fewer_frames_than_it_consumes() {
        let chunk = 1024usize;
        let mut r = BlockResampler::new(96_000, 48_000, 1, chunk).unwrap();
        let total_in = chunk * 8;
        let block = vec![0.25f32; total_in];
        let out_frames = r.process_interleaved(&block).unwrap().len();
        assert!(
            out_frames < total_in,
            "downsampling must yield fewer frames"
        );
    }

    #[test]
    fn interleaving_is_preserved_for_stereo() {
        // Distinct constant per channel so re-interleaving order is checkable.
        let chunk = 256usize;
        let mut r = BlockResampler::new(48_000, 48_000, 2, chunk).unwrap();
        // 1:1 ratio still flows through the resampler; feed many full chunks.
        let frames = chunk * 4;
        let mut block = Vec::with_capacity(frames * 2);
        for _ in 0..frames {
            block.push(1.0f32); // ch0
            block.push(-1.0f32); // ch1
        }
        let out = r.process_interleaved(&block).unwrap();
        assert!(!out.is_empty());
        assert_eq!(out.len() % 2, 0, "stereo output must be frame-aligned");
        // After the startup transient the channels stay separated near ±1.
        // Check the back half (steady state) keeps ch0 ≈ +1, ch1 ≈ -1 in sign.
        let tail = &out[out.len() / 2..];
        for frame in tail.chunks_exact(2) {
            assert!(frame[0] > 0.0, "ch0 should stay positive, got {}", frame[0]);
            assert!(frame[1] < 0.0, "ch1 should stay negative, got {}", frame[1]);
        }
    }

    #[test]
    fn accumulates_across_small_blocks() {
        // Many sub-chunk blocks should still eventually produce output once a
        // full chunk's worth has arrived — the cpal callback won't hand us neat
        // chunk-sized blocks.
        let chunk = 512usize;
        let mut r = BlockResampler::new(44_100, 48_000, 1, chunk).unwrap();
        let mut produced = 0usize;
        for _ in 0..(chunk * 4 / 100 + 1) {
            produced += r.process_interleaved(&vec![0.2f32; 100]).unwrap().len();
        }
        assert!(produced > 0, "small blocks should accumulate into output");
    }

    #[test]
    fn exposes_chunk_frames() {
        let r = BlockResampler::new(44_100, 48_000, 1, 777).unwrap();
        assert_eq!(r.chunk_frames(), 777);
    }
}
