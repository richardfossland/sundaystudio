-- Per-track voice-processing preset (Phase 4.3). NULL = no processing.
-- The id matches dsp::chain::Preset (e.g. 'voice', 'bright-voice', 'broadcast');
-- export applies the chain to the track before mixing.
ALTER TABLE track ADD COLUMN voice_preset TEXT;
