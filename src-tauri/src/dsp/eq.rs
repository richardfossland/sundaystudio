//! 4-band parametric EQ. Each band is an independent biquad you can set to a
//! high-pass, low-pass, bell (peaking) or shelf. Voice presets typically use a
//! high-pass at ~80 Hz and a gentle presence bell around 3 kHz.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::biquad::{Biquad, FilterType};
use super::Effect;

/// Band filter shape, mirrored to TS for the band editor.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/EqBandType.ts")]
#[serde(rename_all = "snake_case")]
pub enum EqBandType {
    HighPass,
    LowPass,
    Bell,
    LowShelf,
    HighShelf,
}

impl EqBandType {
    fn to_filter(self) -> FilterType {
        match self {
            EqBandType::HighPass => FilterType::HighPass,
            EqBandType::LowPass => FilterType::LowPass,
            EqBandType::Bell => FilterType::Peaking,
            EqBandType::LowShelf => FilterType::LowShelf,
            EqBandType::HighShelf => FilterType::HighShelf,
        }
    }
}

/// One EQ band's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/EqBand.ts")]
pub struct EqBand {
    pub enabled: bool,
    pub band_type: EqBandType,
    pub freq: f32,
    pub q: f32,
    pub gain_db: f32,
}

impl EqBand {
    pub const fn bell(freq: f32, q: f32, gain_db: f32) -> Self {
        Self {
            enabled: true,
            band_type: EqBandType::Bell,
            freq,
            q,
            gain_db,
        }
    }
    pub const fn high_pass(freq: f32) -> Self {
        Self {
            enabled: true,
            band_type: EqBandType::HighPass,
            freq,
            q: 0.707,
            gain_db: 0.0,
        }
    }
    fn disabled() -> Self {
        Self {
            enabled: false,
            band_type: EqBandType::Bell,
            freq: 1000.0,
            q: 1.0,
            gain_db: 0.0,
        }
    }
}

/// A 4-band parametric EQ.
#[derive(Debug, Clone)]
pub struct ParametricEq {
    pub bands: [EqBand; 4],
    pub bypass: bool,
    sample_rate: f32,
    filters: [Biquad; 4],
}

impl Default for ParametricEq {
    fn default() -> Self {
        Self {
            // Transparent by default: all bands disabled.
            bands: [EqBand::disabled(); 4],
            bypass: false,
            sample_rate: 48_000.0,
            filters: Default::default(),
        }
    }
}

impl ParametricEq {
    /// Build an EQ from a set of bands (unused slots padded disabled).
    pub fn from_bands(input: &[EqBand]) -> Self {
        let mut bands = [EqBand::disabled(); 4];
        for (slot, b) in bands.iter_mut().zip(input.iter()) {
            *slot = *b;
        }
        Self {
            bands,
            ..Default::default()
        }
    }

    fn redesign(&mut self) {
        for (filter, band) in self.filters.iter_mut().zip(self.bands.iter()) {
            if band.enabled {
                filter.set(
                    band.band_type.to_filter(),
                    self.sample_rate,
                    band.freq,
                    band.q,
                    band.gain_db,
                );
            }
            filter.reset();
        }
    }
}

impl Effect for ParametricEq {
    fn prepare(&mut self, sample_rate: f32) {
        self.sample_rate = sample_rate;
        self.redesign();
    }

    fn process(&mut self, block: &mut [f32]) {
        if self.bypass {
            return;
        }
        for (filter, band) in self.filters.iter_mut().zip(self.bands.iter()) {
            if band.enabled {
                filter.process(block);
            }
        }
    }

    fn reset(&mut self) {
        for f in &mut self.filters {
            f.reset();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::gain_db_at;

    const SR: f32 = 48_000.0;

    #[test]
    fn default_is_transparent() {
        let mut eq = ParametricEq::default();
        eq.prepare(SR);
        assert!(gain_db_at(&mut eq, SR, 1000.0).abs() < 0.5);
    }

    #[test]
    fn highpass_plus_presence_bell_shapes_voice() {
        let mut eq =
            ParametricEq::from_bands(&[EqBand::high_pass(80.0), EqBand::bell(3000.0, 1.0, 6.0)]);
        eq.prepare(SR);
        // Lows rolled off, presence lifted, an untouched mid roughly flat.
        assert!(gain_db_at(&mut eq, SR, 40.0) < -6.0);
        eq.reset();
        assert!(gain_db_at(&mut eq, SR, 3000.0) > 3.0);
        eq.reset();
        assert!(gain_db_at(&mut eq, SR, 700.0).abs() < 1.5);
    }

    #[test]
    fn bypass_passes_through() {
        let mut eq = ParametricEq::from_bands(&[EqBand::bell(1000.0, 1.0, 12.0)]);
        eq.prepare(SR);
        eq.bypass = true;
        assert!(gain_db_at(&mut eq, SR, 1000.0).abs() < 0.5);
    }
}
