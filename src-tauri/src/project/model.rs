//! Project domain model — the tape model (Project → Track / Take → Region).
//!
//! Each struct maps to a table in `migrations/0001_init.sql`. Scalar-only fields
//! derive `sqlx::FromRow` for direct row decoding; `Take` carries a `Vec<String>`
//! (its source track ids), so it is mapped by hand in `store`.
//!
//! Numeric conventions chosen so the generated TypeScript stays `number`: times,
//! positions and durations are `f64` milliseconds; rates/counts are `i32`.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One recording project. One project per `.sqlite` file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[ts(export, export_to = "../../src/lib/bindings/Project.ts")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub sample_rate: i32,
    pub channel_count: i32,
    /// Epoch milliseconds (kept as f64 so TS sees a plain number).
    pub created_at: f64,
}

/// A mixer/timeline track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[ts(export, export_to = "../../src/lib/bindings/Track.ts")]
pub struct Track {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub color: String,
    /// Interface input channel this track records from (None = unassigned).
    pub input_assignment: Option<i32>,
    pub output_assignment: Option<i32>,
    pub gain_db: f64,
    /// -1.0 (hard left) .. +1.0 (hard right).
    pub pan: f64,
    pub mute: bool,
    pub solo: bool,
    pub armed: bool,
    /// Order within the project's track list.
    pub position: i32,
}

/// A raw recording pass. The WAVs live at `takes/{id}/{source_track}.wav`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/Take.ts")]
pub struct Take {
    pub id: String,
    pub project_id: String,
    pub started_at: f64,
    pub duration_ms: f64,
    /// Track ids captured in this take.
    pub source_tracks: Vec<String>,
}

/// A non-destructive reference to a time range within a take, placed on the
/// timeline of a target track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[ts(export, export_to = "../../src/lib/bindings/Region.ts")]
pub struct Region {
    pub id: String,
    pub take_id: String,
    pub source_track_id: String,
    pub target_track_id: String,
    pub start_in_take_ms: f64,
    pub end_in_take_ms: f64,
    pub position_in_timeline_ms: f64,
    pub fade_in_ms: f64,
    pub fade_out_ms: f64,
    pub gain_adjust_db: f64,
}

/// A timeline marker (also used as a podcast chapter on export).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS, sqlx::FromRow)]
#[ts(export, export_to = "../../src/lib/bindings/Marker.ts")]
pub struct Marker {
    pub id: String,
    pub project_id: String,
    pub position_ms: f64,
    pub label: String,
    pub color: String,
}

/// A project plus its tracks and markers — what `project_open` returns so the
/// UI can render the whole project in one round-trip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/ProjectSnapshot.ts")]
pub struct ProjectSnapshot {
    pub project: Project,
    pub tracks: Vec<Track>,
    pub markers: Vec<Marker>,
}

/// A project's takes and the regions placed from them — what `project_timeline`
/// returns so the editor can render the whole timeline in one round-trip. Tracks
/// and markers already come from [`ProjectSnapshot`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/TimelineSnapshot.ts")]
pub struct TimelineSnapshot {
    pub takes: Vec<Take>,
    pub regions: Vec<Region>,
}

/// A recent-projects entry (stored app-side, see `scast`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../../src/lib/bindings/RecentProject.ts")]
pub struct RecentProject {
    pub name: String,
    /// Absolute path to the `.scast` folder.
    pub path: String,
    /// Epoch milliseconds the project was last opened.
    pub last_opened: f64,
}
