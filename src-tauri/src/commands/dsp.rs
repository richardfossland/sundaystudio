//! DSP commands. For Phase 4.1 this exposes the bundled factory presets so the
//! UI can list them. Applying a preset to a track (writing its effect chain to
//! the project) and live processing arrive with the mixer in Phase 4.2.

use crate::dsp::chain::{preset_infos, PresetInfo};
use crate::error::AppResult;

/// The bundled voice-processing presets (Voice / Bright Voice / Warm Voice /
/// Broadcast). All free; the AI "Smart Preset" that picks one is Pro (Phase 4.3).
#[tauri::command]
pub fn dsp_presets() -> AppResult<Vec<PresetInfo>> {
    Ok(preset_infos())
}
