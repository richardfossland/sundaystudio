//! The voice processing chain and factory presets.
//!
//! Signal order is the broadcast standard: gate (clean up the silences) → EQ
//! (shape tone) → de-esser (tame sibilance) → compressor (even dynamics) →
//! saturator (a touch of warmth). Each stage is independently bypassable.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use super::compressor::Compressor;
use super::deesser::DeEsser;
use super::eq::{EqBand, EqBandType, ParametricEq};
use super::gate::Gate;
use super::saturator::Saturator;
use super::Effect;

/// The bundled voice chain. Owns one of each effect.
#[derive(Debug, Clone, Default)]
pub struct VoiceChain {
    pub gate: Gate,
    pub eq: ParametricEq,
    pub deesser: DeEsser,
    pub compressor: Compressor,
    pub saturator: Saturator,
}

impl Effect for VoiceChain {
    fn prepare(&mut self, sample_rate: f32) {
        self.gate.prepare(sample_rate);
        self.eq.prepare(sample_rate);
        self.deesser.prepare(sample_rate);
        self.compressor.prepare(sample_rate);
        self.saturator.prepare(sample_rate);
    }

    fn process(&mut self, block: &mut [f32]) {
        self.gate.process(block);
        self.eq.process(block);
        self.deesser.process(block);
        self.compressor.process(block);
        self.saturator.process(block);
    }

    fn reset(&mut self) {
        self.gate.reset();
        self.eq.reset();
        self.deesser.reset();
        self.compressor.reset();
        self.saturator.reset();
    }
}

const fn shelf(band_type: EqBandType, freq: f32, gain_db: f32) -> EqBand {
    EqBand {
        enabled: true,
        band_type,
        freq,
        q: 0.707,
        gain_db,
    }
}

/// Factory presets shipped with the app (all free; the AI "Smart Preset" that
/// picks one for you is Phase 4.3 / Pro).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Preset {
    Voice,
    BrightVoice,
    WarmVoice,
    Broadcast,
}

impl Preset {
    pub fn id(self) -> &'static str {
        match self {
            Preset::Voice => "voice",
            Preset::BrightVoice => "bright-voice",
            Preset::WarmVoice => "warm-voice",
            Preset::Broadcast => "broadcast",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Preset::Voice => "Voice",
            Preset::BrightVoice => "Bright Voice",
            Preset::WarmVoice => "Warm Voice",
            Preset::Broadcast => "Broadcast",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Preset::Voice => "Clean, natural — a high-pass and gentle dynamics.",
            Preset::BrightVoice => "Adds presence and air, with de-essing to stay smooth.",
            Preset::WarmVoice => "Fuller and rounder; a little low-shelf and tape warmth.",
            Preset::Broadcast => "Radio-ready: present, controlled, consistently loud.",
        }
    }

    pub const ALL: [Preset; 4] = [
        Preset::Voice,
        Preset::BrightVoice,
        Preset::WarmVoice,
        Preset::Broadcast,
    ];

    pub fn from_id(id: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.id() == id)
    }

    /// Materialise the preset into a configured (un-prepared) chain.
    pub fn build(self) -> VoiceChain {
        let mut c = VoiceChain::default();
        match self {
            Preset::Voice => {
                c.gate = Gate::voice(-45.0);
                c.eq = ParametricEq::from_bands(&[
                    EqBand::high_pass(80.0),
                    EqBand::bell(3000.0, 1.0, 3.0),
                ]);
                c.compressor = Compressor::voice(-18.0, 3.0);
            }
            Preset::BrightVoice => {
                c.gate = Gate::voice(-45.0);
                c.eq = ParametricEq::from_bands(&[
                    EqBand::high_pass(90.0),
                    EqBand::bell(4000.0, 1.0, 5.0),
                    shelf(EqBandType::HighShelf, 9000.0, 3.0),
                ]);
                c.deesser = DeEsser::voice(-28.0);
                c.compressor = Compressor::voice(-18.0, 3.0);
            }
            Preset::WarmVoice => {
                c.gate = Gate::voice(-45.0);
                c.eq = ParametricEq::from_bands(&[
                    EqBand::high_pass(70.0),
                    shelf(EqBandType::LowShelf, 200.0, 2.0),
                    EqBand::bell(3000.0, 1.0, 2.0),
                ]);
                c.compressor = Compressor::voice(-20.0, 2.5);
                c.saturator = Saturator::warm(2.0, 0.2);
            }
            Preset::Broadcast => {
                c.gate = Gate::voice(-42.0);
                c.eq = ParametricEq::from_bands(&[
                    EqBand::high_pass(90.0),
                    EqBand::bell(250.0, 1.0, -2.0),
                    EqBand::bell(4000.0, 1.0, 4.0),
                ]);
                c.deesser = DeEsser::voice(-28.0);
                c.compressor = Compressor::voice(-20.0, 4.0);
                c.saturator = Saturator::warm(2.0, 0.15);
            }
        }
        c
    }
}

/// Preset metadata for the UI (the preset picker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/PresetInfo.ts")]
pub struct PresetInfo {
    pub id: String,
    pub label: String,
    pub description: String,
}

/// All factory presets, for `dsp_presets`.
pub fn preset_infos() -> Vec<PresetInfo> {
    Preset::ALL
        .into_iter()
        .map(|p| PresetInfo {
            id: p.id().to_string(),
            label: p.label().to_string(),
            description: p.description().to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::testutil::gain_db_at;

    const SR: f32 = 48_000.0;

    #[test]
    fn presets_have_stable_ids() {
        assert_eq!(preset_infos().len(), 4);
        assert!(Preset::from_id("broadcast").is_some());
        assert!(Preset::from_id("nope").is_none());
    }

    #[test]
    fn every_preset_high_passes_and_stays_finite() {
        for p in Preset::ALL {
            let mut chain = p.build();
            chain.prepare(SR);
            // All voice presets roll off deep lows (high-pass at 70–90 Hz).
            let low = gain_db_at(&mut chain, SR, 40.0);
            assert!(low < -3.0, "{} should cut 40 Hz, got {low}", p.label());

            chain.reset();
            let mut buf = crate::dsp::testutil::sine(1000.0, SR, 4800);
            chain.process(&mut buf);
            assert!(
                buf.iter().all(|s| s.is_finite()),
                "{} produced non-finite output",
                p.label()
            );
        }
    }

    #[test]
    fn default_chain_is_transparent() {
        let mut c = VoiceChain::default();
        c.prepare(SR);
        assert!(gain_db_at(&mut c, SR, 1000.0).abs() < 0.5);
    }
}
