//! Quick-start templates (Phase 2.3).
//!
//! Most users should never start from a blank project — they pick a template
//! that already has the right tracks, colours and input assignments. A template
//! is pure data (defined here, so it's testable); `apply` materialises it into
//! a freshly-created project by inserting its tracks.
//!
//! Mic tracks get a 1-based interface input assignment; music-bed tracks are
//! input-less (`None`). Default per-track effect chains (gate/EQ/compressor)
//! arrive with the DSP effects in Phase 4.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use ts_rs::TS;

use super::store;
use crate::error::AppResult;

/// Track palette, cycled across a template's tracks.
const PALETTE: [&str; 8] = [
    "#D4A73A", // Sunday gold
    "#3A8DD4", // blue
    "#4CB97A", // green
    "#D47A3A", // orange
    "#9B6BD4", // purple
    "#D44A6B", // rose
    "#3AC2C2", // teal
    "#B9A24C", // brass
];

/// One track in a template.
#[derive(Debug, Clone)]
struct SpecTrack {
    name: &'static str,
    /// 1-based interface input channel, or None for a music/bed track.
    input: Option<i32>,
}

impl SpecTrack {
    const fn mic(name: &'static str, input: i32) -> Self {
        Self {
            name,
            input: Some(input),
        }
    }
    const fn bed(name: &'static str) -> Self {
        Self { name, input: None }
    }
}

struct Spec {
    id: &'static str,
    label: &'static str,
    description: &'static str,
    /// Interface input channels the template expects.
    channel_count: i32,
    tracks: &'static [SpecTrack],
}

const TEMPLATES: &[Spec] = &[
    Spec {
        id: "solo",
        label: "Solo podcast",
        description: "One host, a music bed and a jingle slot.",
        channel_count: 1,
        tracks: &[SpecTrack::mic("Host", 1), SpecTrack::bed("Music bed")],
    },
    Spec {
        id: "two-person",
        label: "Two-person conversation",
        description: "Host and guest, music bed, jingle slots.",
        channel_count: 2,
        tracks: &[
            SpecTrack::mic("Host", 1),
            SpecTrack::mic("Guest", 2),
            SpecTrack::bed("Music bed"),
        ],
    },
    Spec {
        id: "panel",
        label: "Panel discussion",
        description: "Three to four voices with auto-mute defaults.",
        channel_count: 4,
        tracks: &[
            SpecTrack::mic("Host", 1),
            SpecTrack::mic("Guest 1", 2),
            SpecTrack::mic("Guest 2", 3),
            SpecTrack::mic("Guest 3", 4),
            SpecTrack::bed("Music bed"),
        ],
    },
    Spec {
        id: "roundtable",
        label: "Roundtable",
        description: "Five to eight voices with smart leveling presets.",
        channel_count: 6,
        tracks: &[
            SpecTrack::mic("Mic 1", 1),
            SpecTrack::mic("Mic 2", 2),
            SpecTrack::mic("Mic 3", 3),
            SpecTrack::mic("Mic 4", 4),
            SpecTrack::mic("Mic 5", 5),
            SpecTrack::mic("Mic 6", 6),
            SpecTrack::bed("Music bed"),
        ],
    },
    Spec {
        id: "sermon",
        label: "Sermon excerpt",
        description: "A single preaching mic with intro/outro music.",
        channel_count: 1,
        tracks: &[
            SpecTrack::mic("Sermon", 1),
            SpecTrack::bed("Intro / outro music"),
        ],
    },
    Spec {
        id: "interview",
        label: "Interview",
        description: "Host and guest on separate tracks for differentiated processing.",
        channel_count: 2,
        tracks: &[
            SpecTrack::mic("Host", 1),
            SpecTrack::mic("Guest", 2),
            SpecTrack::bed("Music bed"),
        ],
    },
    Spec {
        id: "worship-recap",
        label: "Worship recap",
        description: "Music-heavy episode with extra bed tracks.",
        channel_count: 2,
        tracks: &[
            SpecTrack::mic("Host", 1),
            SpecTrack::mic("Co-host", 2),
            SpecTrack::bed("Worship bed"),
            SpecTrack::bed("Music bed"),
        ],
    },
    Spec {
        id: "blank",
        label: "Blank",
        description: "Start from scratch.",
        channel_count: 2,
        tracks: &[],
    },
];

// ── Frontend-facing types (for the template gallery) ──────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../src/lib/bindings/TemplateTrackInfo.ts")]
pub struct TemplateTrackInfo {
    pub name: String,
    pub color: String,
    /// 1-based interface input channel, or null for a music/bed track.
    pub input_assignment: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TS, PartialEq)]
#[ts(export, export_to = "../../src/lib/bindings/TemplateInfo.ts")]
pub struct TemplateInfo {
    pub id: String,
    pub label: String,
    pub description: String,
    pub channel_count: i32,
    /// How many of the tracks are mics (drives the "N mics" badge).
    pub mic_count: i32,
    pub tracks: Vec<TemplateTrackInfo>,
}

fn color_for(i: usize) -> String {
    PALETTE[i % PALETTE.len()].to_string()
}

fn to_info(spec: &Spec) -> TemplateInfo {
    let tracks: Vec<TemplateTrackInfo> = spec
        .tracks
        .iter()
        .enumerate()
        .map(|(i, t)| TemplateTrackInfo {
            name: t.name.to_string(),
            color: color_for(i),
            input_assignment: t.input,
        })
        .collect();
    let mic_count = spec.tracks.iter().filter(|t| t.input.is_some()).count() as i32;
    TemplateInfo {
        id: spec.id.to_string(),
        label: spec.label.to_string(),
        description: spec.description.to_string(),
        channel_count: spec.channel_count,
        mic_count,
        tracks,
    }
}

/// All templates, in gallery order.
pub fn all() -> Vec<TemplateInfo> {
    TEMPLATES.iter().map(to_info).collect()
}

fn find(id: &str) -> Option<&'static Spec> {
    TEMPLATES.iter().find(|s| s.id == id)
}

/// The interface channel count a template expects (for `Project.channel_count`).
pub fn channel_count(id: &str) -> Option<i32> {
    find(id).map(|s| s.channel_count)
}

/// Insert a template's tracks into a just-created project.
pub async fn apply(pool: &SqlitePool, project_id: &str, template_id: &str) -> AppResult<()> {
    let Some(spec) = find(template_id) else {
        return Err(crate::error::AppError::Validation(format!(
            "unknown template: {template_id}"
        )));
    };
    for (i, t) in spec.tracks.iter().enumerate() {
        store::add_track_with(pool, project_id, t.name, &color_for(i), t.input).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ships_eight_templates_with_a_blank() {
        let all = all();
        assert_eq!(all.len(), 8);
        assert!(all.iter().any(|t| t.id == "blank" && t.tracks.is_empty()));
    }

    #[test]
    fn mic_count_counts_only_input_tracks() {
        let two = all().into_iter().find(|t| t.id == "two-person").unwrap();
        assert_eq!(two.mic_count, 2); // host + guest, not the music bed
        assert_eq!(two.tracks.len(), 3);
        assert_eq!(two.channel_count, 2);
    }

    #[tokio::test]
    async fn apply_materialises_tracks_with_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let pool = store::open_pool(&dir.path().join("p.sqlite"))
            .await
            .unwrap();
        let project = store::create_project(&pool, "P", 48_000, 2).await.unwrap();

        apply(&pool, &project.id, "two-person").await.unwrap();

        let tracks = store::list_tracks(&pool, &project.id).await.unwrap();
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].name, "Host");
        assert_eq!(tracks[0].input_assignment, Some(1));
        assert_eq!(tracks[1].input_assignment, Some(2));
        assert_eq!(tracks[2].name, "Music bed");
        assert_eq!(tracks[2].input_assignment, None);
    }

    #[test]
    fn mic_inputs_are_contiguous_1_to_n_within_channel_count() {
        // Every template's mic inputs drive the recorder's interface routing, so
        // they must be a gapless 1..=mic_count with no dupes, and never exceed the
        // channel count the template declares it needs.
        for t in all() {
            let mut inputs: Vec<i32> = t
                .tracks
                .iter()
                .filter_map(|tr| tr.input_assignment)
                .collect();
            inputs.sort_unstable();
            let expected: Vec<i32> = (1..=t.mic_count).collect();
            assert_eq!(
                inputs, expected,
                "template {} mic inputs must be a contiguous 1..={} (got {inputs:?})",
                t.id, t.mic_count
            );
            if let Some(&max) = inputs.last() {
                assert!(
                    max <= t.channel_count,
                    "template {} routes input {max} beyond its channel_count {}",
                    t.id,
                    t.channel_count
                );
            }
        }
    }

    #[test]
    fn template_ids_are_unique() {
        let mut ids: Vec<String> = all().into_iter().map(|t| t.id).collect();
        let count = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), count, "duplicate template id");
    }

    #[test]
    fn channel_count_lookup_matches_gallery() {
        for t in all() {
            assert_eq!(channel_count(&t.id), Some(t.channel_count));
        }
        assert_eq!(channel_count("nope"), None);
    }

    #[tokio::test]
    async fn apply_unknown_template_errors() {
        let dir = tempfile::tempdir().unwrap();
        let pool = store::open_pool(&dir.path().join("p.sqlite"))
            .await
            .unwrap();
        let project = store::create_project(&pool, "P", 48_000, 2).await.unwrap();
        assert!(apply(&pool, &project.id, "nope").await.is_err());
    }
}
